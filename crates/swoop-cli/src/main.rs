//! `swoop`: the Swoop command line — CLI client, headless server, and browser native-messaging
//! host. See `docs/cli.md` for the command reference.

mod cli;
mod commands;
mod error;
mod output;
mod resolve;
mod transport;

use clap::Parser;

use cli::{Cli, Commands};
use error::CliError;

#[tokio::main]
async fn main() {
    let cli = Cli::parse_from(native_host_argv(std::env::args_os().collect()));
    resolve::apply_data_dir_override(&cli.data_dir);

    if let Err(e) = run(cli).await {
        eprintln!("swoop: {e}");
        std::process::exit(e.exit_code());
    }
}

/// Browsers launch a native-messaging host with their own arguments rather than ours: Chromium
/// passes the caller origin (`chrome-extension://<id>/`), Firefox passes the host manifest path
/// and the extension id. When a manifest points straight at this binary, map that invocation to
/// `swoop native-host`.
fn native_host_argv(args: Vec<std::ffi::OsString>) -> Vec<std::ffi::OsString> {
    let first = args.get(1).and_then(|a| a.to_str()).unwrap_or_default();
    let launched_by_browser = first.starts_with("chrome-extension://")
        || (first.ends_with(".json") && args.len() == 3 && std::path::Path::new(first).is_file());
    if launched_by_browser {
        vec![args[0].clone(), "native-host".into()]
    } else {
        args
    }
}

async fn run(mut cli: Cli) -> Result<(), CliError> {
    let json = cli.json;

    if cli.headless {
        return commands::server::run(&cli, cli::ServerArgs::default()).await;
    }

    let command = cli
        .command
        .take()
        .ok_or_else(|| CliError::Usage("no command given. Try `swoop --help`.".to_owned()))?;

    // These own their transport (native-host always talks to the local socket; server runs
    // the engine in-process; update talks to GitHub) and never need a pre-built request client.
    let command = match command {
        Commands::Server(args) => return commands::server::run(&cli, args).await,
        Commands::NativeHost(args) => return commands::native_host::run(&cli, args).await,
        Commands::Update(args) => return commands::update::run(args, json).await,
        other => other,
    };

    let client = resolve::build_client(&cli)?;

    match command {
        Commands::Add(args) => commands::tasks::add(&client, args, json).await,
        Commands::List(args) => commands::tasks::list(&client, args, json).await,
        Commands::Status(args) => commands::tasks::status(&client, args, json).await,
        Commands::Pause(args) => commands::tasks::task_action(&client, args, "pause", json).await,
        Commands::Resume(args) => commands::tasks::task_action(&client, args, "resume", json).await,
        Commands::Retry(args) => commands::tasks::task_action(&client, args, "retry", json).await,
        Commands::Restart(args) => {
            commands::tasks::task_action(&client, args, "restart", json).await
        }
        Commands::Cancel(args) => commands::tasks::task_action(&client, args, "cancel", json).await,
        Commands::Remove(args) => commands::tasks::remove(&client, args, json).await,
        Commands::PauseAll => commands::tasks::pause_all(&client, json).await,
        Commands::ResumeAll => commands::tasks::resume_all(&client, json).await,
        Commands::Watch(args) => commands::watch::run(&client, args).await,
        Commands::Queue(args) => commands::queue::run(&client, args, json).await,
        Commands::Limit(args) => commands::bandwidth::limit(&client, args, json).await,
        Commands::Mode(args) => commands::bandwidth::mode(&client, args, json).await,
        Commands::History(args) => commands::history::run(&client, args, json).await,
        Commands::Export(args) => commands::importexport::export(&client, args).await,
        Commands::Import(args) => commands::importexport::import(&client, args, json).await,
        Commands::Pair => commands::pair::run(&client, json).await,
        Commands::Devices(args) => commands::devices::run(&client, args, json).await,
        Commands::Diagnostics(args) => commands::diagnostics::run(&client, args, json).await,
        Commands::Server(_) | Commands::NativeHost(_) | Commands::Update(_) => {
            unreachable!("handled above before the client was built")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::native_host_argv;

    #[test]
    fn browser_launch_maps_to_native_host() {
        let argv = |v: &[&str]| {
            v.iter()
                .map(Into::into)
                .collect::<Vec<std::ffi::OsString>>()
        };
        assert_eq!(
            native_host_argv(argv(&["swoop", "chrome-extension://abc/"])),
            argv(&["swoop", "native-host"])
        );
        assert_eq!(
            native_host_argv(argv(&["swoop", "list"])),
            argv(&["swoop", "list"])
        );
    }
}
