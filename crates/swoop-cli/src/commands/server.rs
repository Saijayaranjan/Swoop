//! `swoop server` / `swoop --headless`: run the engine in-process with the local (and
//! optionally remote) API server.

use swoop_runtime::paths::AppPaths;
use swoop_services::bootstrap::{self, EngineConfig};

use crate::cli::{Cli, ServerArgs};
use crate::error::{CliError, CliResult};

fn domain_err(e: impl std::fmt::Display) -> CliError {
    CliError::Other(anyhow::anyhow!(e.to_string()))
}

pub async fn run(cli: &Cli, args: ServerArgs) -> CliResult<()> {
    init_logging(cli.log_level.as_deref());

    let paths = AppPaths::resolve();
    let config = EngineConfig {
        data_dir: cli.data_dir.clone(),
        headless: true,
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        skip_instance_lock: false,
        download_dir: args.download_dir.clone(),
    };

    tracing::info!(data_dir = %paths.data_dir.display(), "starting swoop engine");
    let engine = bootstrap::start(config).await.map_err(domain_err)?;

    let socket_path = cli.socket.clone().unwrap_or_else(|| paths.socket());
    if args.no_tls && args.remote.is_none() {
        tracing::warn!("--no-tls has no effect without --remote");
    }
    if let Some(bind) = args.remote {
        if args.no_tls {
            tracing::warn!(
                "remote listener starting WITHOUT TLS (--no-tls) on {bind} — traffic is \
                 unencrypted; only do this behind a trusted reverse proxy"
            );
        }
    }
    let remote = args.remote.map(|bind| swoop_server::RemoteOptions {
        bind,
        tls: !args.no_tls,
        tls_dir: paths.tls_dir(),
        allowed_origins: Vec::new(),
        rate_limit_per_minute: 300,
    });

    let opts = swoop_server::ServerOptions {
        socket_path: Some(socket_path),
        local_tcp: args.listen,
        remote,
        serve_web_ui: true,
    };

    let handle = swoop_server::start(engine.clone(), opts)
        .await
        .map_err(domain_err)?;

    if let Some(sp) = &handle.socket_path {
        tracing::info!(path = %sp.display(), "unix socket listening");
    }
    if let Some(a) = handle.local_addr {
        tracing::info!(addr = %a, "local TCP listening");
    }
    if let Some(a) = handle.remote_addr {
        tracing::info!(addr = %a, "remote TCP listening");
    }
    tracing::info!("swoop server ready");
    let update_notifier = tokio::spawn(notify_updates(engine.clone()));

    wait_for_shutdown_signal().await;

    tracing::info!("shutting down");
    update_notifier.abort();
    engine.shutdown().await;
    handle.shutdown().await;
    Ok(())
}

/// Headless servers only *notify* about updates (log line, notification, webhooks); they never
/// download or install one. Checks run shortly after start and then daily, when enabled.
async fn notify_updates(engine: swoop_services::SharedEngine) {
    const DAY_MS: i64 = 24 * 60 * 60 * 1000;
    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    loop {
        let s = engine.settings();
        let due = s
            .updates
            .last_check_at
            .is_none_or(|t| swoop_domain::Millis::now().0 - t.0 >= DAY_MS);
        if s.updates.check_automatically && due {
            match engine.check_for_updates().await {
                Ok(u) if u.available => tracing::info!(
                    version = u.latest_version.as_deref().unwrap_or("?"),
                    "a newer Swoop release is available (update the host manually)"
                ),
                Ok(u) => tracing::debug!(status = %u.status, "update check"),
                Err(e) => tracing::debug!(error = %e, "update check skipped"),
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(60 * 60)).await;
    }
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn init_logging(level: Option<&str>) {
    use tracing_subscriber::EnvFilter;
    let filter = match level {
        Some(l) => EnvFilter::new(l),
        None => EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}
