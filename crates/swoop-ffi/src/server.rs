//! The in-app API servers (`swoop-server`): the local listener (Unix socket + loopback TCP) so the
//! CLI and the browser native host talk to the in-app engine, and the optional remote listener,
//! started/stopped as `settings.remote` changes.
//!
//! Without the `local-api` feature the supervisor is inert (the engine still works in-process).

#[cfg(feature = "local-api")]
pub use imp::ServerSupervisor;
#[cfg(not(feature = "local-api"))]
pub use inert::ServerSupervisor;

/// What the app shows in Settings → Remote / Advanced.
#[derive(Clone, Debug, Default)]
pub struct ServerInfo {
    pub socket_path: Option<String>,
    pub local_address: Option<String>,
    pub remote_address: Option<String>,
    pub tls_fingerprint: Option<String>,
}

#[cfg(feature = "local-api")]
mod imp {
    use super::ServerInfo;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::path::PathBuf;
    use swoop_domain::settings::{RemoteSettings, Settings};
    use swoop_runtime::paths::AppPaths;
    use swoop_server::{RemoteOptions, ServerHandle, ServerOptions};
    use swoop_services::SharedEngine;
    use tokio::sync::Mutex;

    /// `sizeof(sockaddr_un.sun_path)` on macOS/BSD.
    const MAX_SOCKET_PATH: usize = 104;

    /// The parts of `RemoteSettings` that require restarting the remote listener.
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct RemoteKey {
        bind_address: String,
        port: u16,
        tls: bool,
        allowed_origins: Vec<String>,
        rate_limit_per_minute: u32,
    }

    impl From<&RemoteSettings> for RemoteKey {
        fn from(r: &RemoteSettings) -> Self {
            Self {
                bind_address: r.bind_address.clone(),
                port: r.port,
                tls: r.tls,
                allowed_origins: r.allowed_origins.clone(),
                rate_limit_per_minute: r.rate_limit_per_minute,
            }
        }
    }

    pub struct ServerSupervisor {
        api: SharedEngine,
        paths: AppPaths,
        local: Mutex<Option<ServerHandle>>,
        remote: Mutex<Option<(RemoteKey, ServerHandle)>>,
        info: parking_lot::Mutex<ServerInfo>,
    }

    impl ServerSupervisor {
        pub fn new(api: SharedEngine, data_dir: PathBuf) -> Self {
            Self {
                api,
                paths: AppPaths {
                    config_dir: data_dir.clone(),
                    cache_dir: data_dir.join("cache"),
                    log_dir: data_dir.join("logs"),
                    data_dir,
                },
                local: Mutex::new(None),
                remote: Mutex::new(None),
                info: parking_lot::Mutex::new(ServerInfo::default()),
            }
        }

        pub fn info(&self) -> ServerInfo {
            self.info.lock().clone()
        }

        /// Start the Unix socket (+ loopback TCP on `settings.remote.local_port` when free).
        pub async fn start_local(&self) {
            let mut slot = self.local.lock().await;
            if slot.is_some() {
                return;
            }
            let socket = self.paths.socket();
            if socket.as_os_str().len() >= MAX_SOCKET_PATH {
                tracing::warn!(
                    path = %socket.display(),
                    "socket path exceeds the {MAX_SOCKET_PATH}-byte sun_path limit; the CLI and \
                     native host cannot reach this engine (use a shorter data directory)"
                );
            }
            let port = self.api.settings().remote.local_port;
            let tcp = (port != 0).then(|| SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port));
            let opts = |local_tcp| ServerOptions {
                socket_path: Some(socket.clone()),
                local_tcp,
                remote: None,
                serve_web_ui: true,
            };
            let handle = match swoop_server::start(self.api.clone(), opts(tcp)).await {
                Ok(h) => Ok(h),
                Err(e) if tcp.is_some() => {
                    // Most likely the port is taken (a headless engine?); the socket is what the CLI
                    // and native host use, so keep going without TCP.
                    tracing::warn!(error = %e, "local TCP API unavailable; starting the socket only");
                    swoop_server::start(self.api.clone(), opts(None)).await
                }
                Err(e) => Err(e),
            };
            match handle {
                Ok(h) => {
                    let mut info = self.info.lock();
                    info.socket_path = h.socket_path.as_deref().map(|p| p.display().to_string());
                    info.local_address = h.local_addr.map(|a| a.to_string());
                    *slot = Some(h);
                }
                Err(e) => tracing::error!(error = %e, "local API could not start"),
            }
        }

        /// Bring the remote listener in line with `settings.remote` (start, stop or restart).
        pub async fn apply_remote(&self, settings: &Settings) {
            let r = &settings.remote;
            let wanted = r.enabled.then(|| RemoteKey::from(r));
            let mut slot = self.remote.lock().await;
            if slot.as_ref().map(|(k, _)| k) == wanted.as_ref() {
                return;
            }
            if let Some((_, h)) = slot.take() {
                h.shutdown().await;
                let mut info = self.info.lock();
                info.remote_address = None;
                info.tls_fingerprint = None;
                tracing::info!("remote API stopped");
            }
            let Some(key) = wanted else {
                return;
            };
            let ip: IpAddr = match key.bind_address.trim().parse() {
                Ok(ip) => ip,
                Err(_) => {
                    tracing::error!(bind = %key.bind_address, "invalid remote bind address");
                    return;
                }
            };
            let opts = ServerOptions {
                socket_path: None,
                local_tcp: None,
                remote: Some(RemoteOptions {
                    bind: SocketAddr::new(ip, key.port),
                    tls: key.tls,
                    tls_dir: self.paths.tls_dir(),
                    allowed_origins: key.allowed_origins.clone(),
                    rate_limit_per_minute: key.rate_limit_per_minute,
                }),
                serve_web_ui: true,
            };
            match swoop_server::start(self.api.clone(), opts).await {
                Ok(h) => {
                    {
                        let mut info = self.info.lock();
                        info.remote_address = h.remote_addr.map(|a| a.to_string());
                        info.tls_fingerprint = h.tls_fingerprint.clone();
                    }
                    *slot = Some((key, h));
                }
                Err(e) => tracing::error!(error = %e, "remote API could not start"),
            }
        }

        pub async fn shutdown(&self) {
            if let Some((_, h)) = self.remote.lock().await.take() {
                h.shutdown().await;
            }
            if let Some(h) = self.local.lock().await.take() {
                h.shutdown().await;
            }
            *self.info.lock() = ServerInfo::default();
        }
    }
}

#[cfg(not(feature = "local-api"))]
mod inert {
    use super::ServerInfo;
    use std::path::PathBuf;
    use swoop_domain::settings::Settings;
    use swoop_services::SharedEngine;

    pub struct ServerSupervisor;

    impl ServerSupervisor {
        pub fn new(_api: SharedEngine, _data_dir: PathBuf) -> Self {
            Self
        }
        pub fn info(&self) -> ServerInfo {
            ServerInfo::default()
        }
        pub async fn start_local(&self) {
            tracing::info!("built without the local-api feature; no API server");
        }
        pub async fn apply_remote(&self, _settings: &Settings) {}
        pub async fn shutdown(&self) {}
    }
}
