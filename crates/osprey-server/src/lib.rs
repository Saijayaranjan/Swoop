//! osprey-server — the REST + WebSocket API over [`osprey_services::EngineApi`], plus the embedded
//! remote web UI. See `docs/api/rest.md`, `docs/api/websocket.md` and
//! `docs/security/threat-model.md`.
//!
//! Three listeners share one router shape:
//!
//! * a Unix socket (mode 0600, same-uid peers only) for the CLI and native-messaging host,
//! * an optional loopback TCP listener (refused on non-loopback addresses),
//! * an optional remote listener (TLS with a persisted self-signed certificate) for paired devices.
//!
//! The local token is accepted (as `admin`) only on the local listeners; paired-device tokens are
//! accepted everywhere with their scopes. Every route's required scope lives in one table
//! ([`auth::policy`]) so authorization is auditable in one place.

mod auth;
mod error;
mod extract;
mod routes;
mod state;
mod tls;
#[cfg(unix)]
mod unix;
mod web;
mod ws;

use osprey_domain::{DomainError, DomainResult};
use osprey_services::SharedEngine;
use state::{AppState, RouterConfig};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub use error::ApiError;

/// Which listener a router serves. Decides whether the local token is accepted and which
/// restrictions (exec automations, archives, platform events) apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ListenerKind {
    /// Unix socket or loopback TCP.
    Local,
    /// The LAN/Internet listener for paired devices.
    Remote,
}

/// What to start.
#[derive(Clone, Debug)]
pub struct ServerOptions {
    /// Unix socket path (ignored on non-Unix platforms).
    pub socket_path: Option<PathBuf>,
    /// Loopback TCP address; must be a loopback IP.
    pub local_tcp: Option<SocketAddr>,
    /// Remote listener, off unless set.
    pub remote: Option<RemoteOptions>,
    /// Serve the embedded web UI at `/`.
    pub serve_web_ui: bool,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            socket_path: None,
            local_tcp: None,
            remote: None,
            serve_web_ui: true,
        }
    }
}

/// Remote listener configuration.
#[derive(Clone, Debug)]
pub struct RemoteOptions {
    pub bind: SocketAddr,
    pub tls: bool,
    /// Where `cert.pem` / `key.pem` are persisted (created on first start, reused afterwards).
    pub tls_dir: PathBuf,
    /// Extra browser origins allowed to call the API (exact match, e.g. `https://nas.local:8443`).
    pub allowed_origins: Vec<String>,
    /// Per-token requests per minute; 0 disables the limit.
    pub rate_limit_per_minute: u32,
}

impl Default for RemoteOptions {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([0, 0, 0, 0], 41780)),
            tls: true,
            tls_dir: osprey_runtime::paths::AppPaths::resolve().tls_dir(),
            allowed_origins: Vec::new(),
            rate_limit_per_minute: 300,
        }
    }
}

/// A running server. Dropping it does **not** stop the listeners; call [`ServerHandle::shutdown`].
#[derive(Debug)]
pub struct ServerHandle {
    pub socket_path: Option<PathBuf>,
    pub local_addr: Option<SocketAddr>,
    pub remote_addr: Option<SocketAddr>,
    /// Lower-case hex SHA-256 of the remote listener's DER certificate (TLS only).
    pub tls_fingerprint: Option<String>,
    shutdown: CancellationToken,
    remote_handle: Option<axum_server::Handle>,
    tasks: Vec<JoinHandle<()>>,
}

impl ServerHandle {
    /// Stop accepting connections, close WebSockets, wait (bounded) for in-flight requests and
    /// remove the Unix socket.
    pub async fn shutdown(self) {
        self.shutdown.cancel();
        if let Some(h) = &self.remote_handle {
            h.graceful_shutdown(Some(Duration::from_secs(5)));
        }
        for mut t in self.tasks {
            if tokio::time::timeout(Duration::from_secs(10), &mut t)
                .await
                .is_err()
            {
                t.abort();
            }
        }
        if let Some(p) = &self.socket_path {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// Build the API + web UI router for one listener kind with default options (tests, embedding).
/// The remote variant uses the engine's `settings.remote` rate limit and allowed origins.
pub fn router(engine: SharedEngine, ctx: ListenerKind) -> axum::Router {
    let settings = engine.settings();
    let cfg = RouterConfig {
        kind: ctx,
        allowed_origins: Vec::new(),
        rate_limit_per_minute: match ctx {
            ListenerKind::Local => 0,
            ListenerKind::Remote => settings.remote.rate_limit_per_minute,
        },
        tls_fingerprint: None,
        serve_web_ui: true,
    };
    routes::build(AppState::new(engine, cfg, CancellationToken::new()))
}

/// Start the configured listeners. Returns once every listener is bound; if any listener fails
/// to bind, the ones already started are stopped again.
pub async fn start(engine: SharedEngine, opts: ServerOptions) -> DomainResult<ServerHandle> {
    let mut handle = ServerHandle {
        socket_path: None,
        local_addr: None,
        remote_addr: None,
        tls_fingerprint: None,
        shutdown: CancellationToken::new(),
        remote_handle: None,
        tasks: Vec::new(),
    };
    match populate(&mut handle, engine, opts).await {
        Ok(()) => {
            tracing::info!(
                socket = ?handle.socket_path,
                local = ?handle.local_addr,
                remote = ?handle.remote_addr,
                "API server started"
            );
            Ok(handle)
        }
        Err(e) => {
            handle.shutdown().await;
            Err(e)
        }
    }
}

async fn populate(
    handle: &mut ServerHandle,
    engine: SharedEngine,
    opts: ServerOptions,
) -> DomainResult<()> {
    let shutdown = handle.shutdown.clone();

    // Prepare TLS first so the local listeners can report the fingerprint in pairing info.
    let tls_material = match &opts.remote {
        Some(r) if r.tls => {
            let dir = r.tls_dir.clone();
            let bind_ip = r.bind.ip();
            let m = tokio::task::spawn_blocking(move || tls::load_or_create(&dir, bind_ip))
                .await
                .map_err(|e| DomainError::Internal(format!("tls task: {e}")))??;
            handle.tls_fingerprint = Some(m.fingerprint.clone());
            Some(m)
        }
        _ => None,
    };

    let local_cfg = |tls_fingerprint: Option<String>| RouterConfig {
        kind: ListenerKind::Local,
        allowed_origins: opts
            .remote
            .as_ref()
            .map(|r| r.allowed_origins.clone())
            .unwrap_or_default(),
        rate_limit_per_minute: 0,
        tls_fingerprint,
        serve_web_ui: opts.serve_web_ui,
    };

    // ----- Unix socket -----
    #[cfg(unix)]
    if let Some(path) = &opts.socket_path {
        let listener = unix::bind(path)?;
        let app = routes::build(AppState::new(
            engine.clone(),
            local_cfg(handle.tls_fingerprint.clone()),
            shutdown.clone(),
        ));
        let token = shutdown.clone();
        handle.tasks.push(tokio::spawn(async move {
            let r = axum::serve(listener, app.into_make_service())
                .with_graceful_shutdown(token.cancelled_owned())
                .await;
            if let Err(e) = r {
                tracing::error!(error = %e, "unix socket server stopped");
            }
        }));
        handle.socket_path = Some(path.clone());
    }
    #[cfg(not(unix))]
    if opts.socket_path.is_some() {
        tracing::warn!("unix sockets are unavailable on this platform; ignoring socket_path");
    }

    // ----- loopback TCP -----
    if let Some(addr) = opts.local_tcp {
        if !addr.ip().is_loopback() {
            return Err(DomainError::Validation(format!(
                "local API address {addr} is not a loopback address; use the remote listener"
            )));
        }
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| DomainError::Unavailable(format!("cannot bind {addr}: {e}")))?;
        let bound = listener
            .local_addr()
            .map_err(|e| DomainError::Internal(e.to_string()))?;
        let app = routes::build(AppState::new(
            engine.clone(),
            local_cfg(handle.tls_fingerprint.clone()),
            shutdown.clone(),
        ));
        let token = shutdown.clone();
        handle.tasks.push(tokio::spawn(async move {
            let r = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(token.cancelled_owned())
            .await;
            if let Err(e) = r {
                tracing::error!(error = %e, "local TCP server stopped");
            }
        }));
        handle.local_addr = Some(bound);
    }

    // ----- remote -----
    if let Some(r) = &opts.remote {
        let std_listener = std::net::TcpListener::bind(r.bind)
            .map_err(|e| DomainError::Unavailable(format!("cannot bind {}: {e}", r.bind)))?;
        std_listener
            .set_nonblocking(true)
            .map_err(|e| DomainError::Internal(e.to_string()))?;
        let bound = std_listener
            .local_addr()
            .map_err(|e| DomainError::Internal(e.to_string()))?;
        let cfg = RouterConfig {
            kind: ListenerKind::Remote,
            allowed_origins: r.allowed_origins.clone(),
            rate_limit_per_minute: r.rate_limit_per_minute,
            tls_fingerprint: handle.tls_fingerprint.clone(),
            serve_web_ui: opts.serve_web_ui,
        };
        let app = routes::build(AppState::new(engine.clone(), cfg, shutdown.clone()));
        let make = app.into_make_service_with_connect_info::<SocketAddr>();
        match tls_material {
            Some(m) => {
                let config = tls::server_config(&m)?;
                let server_handle = axum_server::Handle::new();
                let server = axum_server::from_tcp_rustls(
                    std_listener,
                    axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(config)),
                )
                .handle(server_handle.clone());
                handle.tasks.push(tokio::spawn(async move {
                    if let Err(e) = server.serve(make).await {
                        tracing::error!(error = %e, "remote TLS server stopped");
                    }
                }));
                handle.remote_handle = Some(server_handle);
            }
            None => {
                tracing::warn!(
                    addr = %bound,
                    "remote API listening WITHOUT TLS; tokens travel in clear text"
                );
                let listener = tokio::net::TcpListener::from_std(std_listener)
                    .map_err(|e| DomainError::Internal(e.to_string()))?;
                let token = shutdown.clone();
                handle.tasks.push(tokio::spawn(async move {
                    let r = axum::serve(listener, make)
                        .with_graceful_shutdown(token.cancelled_owned())
                        .await;
                    if let Err(e) = r {
                        tracing::error!(error = %e, "remote server stopped");
                    }
                }));
            }
        }
        handle.remote_addr = Some(bound);
    }

    Ok(())
}
