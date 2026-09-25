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
    let cli = Cli::parse();
    resolve::apply_data_dir_override(&cli.data_dir);

    if let Err(e) = run(cli).await {
        eprintln!("swoop: {e}");
        std::process::exit(e.exit_code());
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

    // These two own their transport (native-host always talks to the local socket; server runs
    // the engine in-process) and never need a pre-built request client.
    let command = match command {
        Commands::Server(args) => return commands::server::run(&cli, args).await,
        Commands::NativeHost(args) => return commands::native_host::run(&cli, args).await,
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
        Commands::Server(_) | Commands::NativeHost(_) => {
            unreachable!("handled above before the client was built")
        }
    }
}
