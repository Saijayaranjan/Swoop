//! Command line surface (`clap` derive). See `docs/cli.md` for the user-facing reference.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;

use clap::{Parser, Subcommand};

/// The `osprey` command line: CLI client, headless server, native messaging host.
#[derive(Parser, Debug)]
#[command(name = "osprey", version, about, propagate_version = true)]
pub struct Cli {
    /// Print machine-readable JSON instead of human-readable tables.
    #[arg(long, global = true)]
    pub json: bool,

    /// Override the Osprey data directory (also settable via `OSPREY_DATA_DIR`).
    #[arg(long, global = true, value_name = "DIR")]
    pub data_dir: Option<PathBuf>,

    /// Path to the local Unix socket (defaults to `<data-dir>/osprey.sock`).
    #[arg(long, global = true, value_name = "PATH")]
    pub socket: Option<PathBuf>,

    /// Talk to a remote/local HTTP(S) server instead of the Unix socket.
    #[arg(long, global = true, value_name = "URL")]
    pub url: Option<String>,

    /// Bearer token for `--url`. Required unless the server has no auth configured.
    #[arg(long, global = true, value_name = "TOKEN")]
    pub token: Option<String>,

    /// Log verbosity for `osprey server` / `osprey --headless` (also `RUST_LOG`).
    #[arg(long, global = true, value_name = "LEVEL")]
    pub log_level: Option<String>,

    /// Shorthand for `osprey server` with default options.
    #[arg(long)]
    pub headless: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

/// A pair of `Key: Value` HTTP headers passed via `--header`.
#[derive(Clone, Debug)]
pub struct HeaderArg {
    pub name: String,
    pub value: String,
}

impl FromStr for HeaderArg {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (name, value) = s
            .split_once(':')
            .ok_or_else(|| format!("expected `Key: Value`, got `{s}`"))?;
        Ok(HeaderArg {
            name: name.trim().to_owned(),
            value: value.trim().to_owned(),
        })
    }
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Add one or more downloads (URL, magnet URI, or a local .torrent file).
    Add(AddArgs),
    /// List tasks.
    List(ListArgs),
    /// Show global stats, or a single task's detail.
    Status(StatusArgs),
    /// Pause tasks.
    Pause(IdsArgs),
    /// Resume tasks.
    Resume(IdsArgs),
    /// Retry failed tasks, keeping partial data.
    Retry(IdsArgs),
    /// Restart tasks from scratch, discarding partial data.
    Restart(IdsArgs),
    /// Cancel tasks.
    Cancel(IdsArgs),
    /// Remove tasks.
    Remove(RemoveArgs),
    /// Pause every active task.
    PauseAll,
    /// Resume every paused task.
    ResumeAll,
    /// Watch live progress (all tasks, or one task) over the WebSocket event stream.
    Watch(WatchArgs),
    /// Manage queues.
    Queue(QueueArgs),
    /// Set explicit global bandwidth limits.
    Limit(LimitArgs),
    /// Set the global traffic mode.
    Mode(ModeArgs),
    /// Show completed/failed task history.
    History(HistoryArgs),
    /// Export tasks/history/settings as a JSON bundle (write to stdout).
    Export(ExportArgs),
    /// Import a JSON bundle previously produced by `osprey export`.
    Import(ImportArgs),
    /// Start device pairing and print the code + a QR code.
    Pair,
    /// List paired devices, or revoke one.
    Devices(DevicesArgs),
    /// Show full diagnostics for a task.
    Diagnostics(DiagnosticsArgs),
    /// Run the engine in-process with the local (and optionally remote) API server.
    Server(ServerArgs),
    /// Run the browser native-messaging host relay (invoked by Chrome/Firefox, not humans).
    NativeHost(NativeHostArgs),
}

#[derive(clap::Args, Debug)]
pub struct AddArgs {
    /// URLs, magnet URIs, or paths to local .torrent files. Each becomes its own task.
    pub sources: Vec<String>,
    /// Destination directory.
    #[arg(long, value_name = "DIR")]
    pub dir: Option<PathBuf>,
    /// Override the file/task name.
    #[arg(long)]
    pub name: Option<String>,
    /// Queue id or name.
    #[arg(long)]
    pub queue: Option<String>,
    /// Number of connections.
    #[arg(long)]
    pub connections: Option<u8>,
    /// Download bandwidth limit in bytes/sec.
    #[arg(long)]
    pub limit: Option<u64>,
    /// Extra HTTP header, `Key: Value`. Repeatable.
    #[arg(long = "header", value_name = "KEY:VALUE")]
    pub headers: Vec<HeaderArg>,
    /// Expected checksum, `algo:hex` (md5, sha1, sha256, sha512, blake3).
    #[arg(long)]
    pub checksum: Option<String>,
    /// Add in `Pending` state instead of starting immediately.
    #[arg(long)]
    pub paused: bool,
    /// Path to a local .torrent file (alternative to a positional source).
    #[arg(long, value_name = "FILE")]
    pub torrent: Option<PathBuf>,
    /// A magnet URI (alternative to a positional source).
    #[arg(long, value_name = "URI")]
    pub magnet: Option<String>,
}

#[derive(clap::Args, Debug)]
pub struct ListArgs {
    /// Filter by state (repeatable): pending, queued, downloading, paused, completed, failed, …
    #[arg(long = "state")]
    pub state: Vec<String>,
    /// Filter by queue id or name.
    #[arg(long)]
    pub queue: Option<String>,
    /// Free-text search over name/URL/domain/tags.
    #[arg(long)]
    pub search: Option<String>,
    /// Maximum number of rows.
    #[arg(long)]
    pub limit: Option<u32>,
}

#[derive(clap::Args, Debug)]
pub struct StatusArgs {
    /// Task id. Omit for global stats.
    pub id: Option<String>,
}

#[derive(clap::Args, Debug)]
pub struct IdsArgs {
    /// One or more task ids.
    #[arg(required = true)]
    pub ids: Vec<String>,
}

#[derive(clap::Args, Debug)]
pub struct RemoveArgs {
    /// One or more task ids.
    #[arg(required = true)]
    pub ids: Vec<String>,
    /// Also delete the downloaded file(s).
    #[arg(long)]
    pub delete_file: bool,
}

#[derive(clap::Args, Debug)]
pub struct WatchArgs {
    /// Only watch this task. Omit to watch everything.
    pub id: Option<String>,
}

#[derive(clap::Args, Debug)]
pub struct QueueArgs {
    #[command(subcommand)]
    pub command: QueueCommand,
}

#[derive(Subcommand, Debug)]
pub enum QueueCommand {
    /// List queues with their summaries.
    List,
    /// Create a queue.
    Create(QueueCreateArgs),
    /// Pause a queue.
    Pause(QueueIdArgs),
    /// Resume a queue.
    Resume(QueueIdArgs),
    /// Delete a queue.
    Delete(QueueIdArgs),
}

#[derive(clap::Args, Debug)]
pub struct QueueCreateArgs {
    pub name: String,
    /// Max concurrently active tasks (0 = unlimited).
    #[arg(long, default_value_t = 0)]
    pub max: u32,
}

#[derive(clap::Args, Debug)]
pub struct QueueIdArgs {
    pub id: String,
}

#[derive(clap::Args, Debug)]
pub struct LimitArgs {
    /// Download limit in bytes/sec (0 = unlimited).
    #[arg(long)]
    pub down: Option<u64>,
    /// Upload limit in bytes/sec (0 = unlimited).
    #[arg(long)]
    pub up: Option<u64>,
}

#[derive(clap::Args, Debug)]
pub struct ModeArgs {
    pub mode: TrafficModeArg,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum TrafficModeArg {
    Unlimited,
    Balanced,
    Browsing,
    Custom,
}

impl TrafficModeArg {
    pub fn as_str(self) -> &'static str {
        match self {
            TrafficModeArg::Unlimited => "unlimited",
            TrafficModeArg::Balanced => "balanced",
            TrafficModeArg::Browsing => "browsing",
            TrafficModeArg::Custom => "custom",
        }
    }
}

#[derive(clap::Args, Debug)]
pub struct HistoryArgs {
    /// Free-text search.
    #[arg(long)]
    pub search: Option<String>,
    /// Maximum number of rows.
    #[arg(long)]
    pub limit: Option<u32>,
}

#[derive(clap::Args, Debug)]
pub struct ExportArgs {
    /// Include tasks.
    #[arg(long)]
    pub tasks: bool,
    /// Include history.
    #[arg(long)]
    pub history: bool,
}

#[derive(clap::Args, Debug)]
pub struct ImportArgs {
    /// Path to a JSON bundle produced by `osprey export`.
    pub file: PathBuf,
    /// Overwrite existing entries with the same id.
    #[arg(long)]
    pub overwrite: bool,
}

#[derive(clap::Args, Debug)]
pub struct DevicesArgs {
    #[command(subcommand)]
    pub command: Option<DevicesCommand>,
}

#[derive(Subcommand, Debug)]
pub enum DevicesCommand {
    /// Revoke a paired device.
    Revoke(QueueIdArgs),
}

#[derive(clap::Args, Debug)]
pub struct DiagnosticsArgs {
    pub id: String,
}

#[derive(clap::Args, Debug, Default)]
pub struct ServerArgs {
    /// Also listen on loopback TCP, e.g. `127.0.0.1:41779`.
    #[arg(long, value_name = "ADDR")]
    pub listen: Option<SocketAddr>,
    /// Enable the remote (LAN/Internet) listener, e.g. `0.0.0.0:41780`. TLS is on by default.
    #[arg(long, value_name = "ADDR")]
    pub remote: Option<SocketAddr>,
    /// Disable TLS on the remote listener. Dangerous: only ever use behind a trusted proxy.
    #[arg(long)]
    pub no_tls: bool,
    /// Override the default download directory.
    #[arg(long, value_name = "DIR")]
    pub download_dir: Option<PathBuf>,
}

#[derive(clap::Args, Debug)]
pub struct NativeHostArgs {
    /// Install the native-messaging host manifest for a browser instead of running the relay.
    #[arg(long, value_name = "BROWSER")]
    pub install_manifest: Option<BrowserArg>,
    /// The published extension id, required with `--install-manifest`.
    #[arg(long, value_name = "ID")]
    pub extension_id: Option<String>,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum BrowserArg {
    Chrome,
    Chromium,
    Edge,
    Brave,
    Firefox,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        let mut full = vec!["osprey"];
        full.extend_from_slice(args);
        Cli::try_parse_from(full).expect("should parse")
    }

    #[test]
    fn global_flags_apply_before_the_subcommand() {
        let cli = parse(&["--json", "--url", "https://host:41780", "--token", "abc", "status"]);
        assert!(cli.json);
        assert_eq!(cli.url.as_deref(), Some("https://host:41780"));
        assert_eq!(cli.token.as_deref(), Some("abc"));
        assert!(matches!(cli.command, Some(Commands::Status(StatusArgs { id: None }))));
    }

    #[test]
    fn add_collects_multiple_sources_and_options() {
        let cli = parse(&[
            "add",
            "https://example.com/a.zip",
            "https://example.com/b.zip",
            "--dir",
            "/tmp/downloads",
            "--connections",
            "8",
            "--header",
            "Authorization: Bearer xyz",
            "--paused",
        ]);
        let Some(Commands::Add(add)) = cli.command else {
            panic!("expected Add");
        };
        assert_eq!(add.sources.len(), 2);
        assert_eq!(add.dir.as_deref(), Some(std::path::Path::new("/tmp/downloads")));
        assert_eq!(add.connections, Some(8));
        assert!(add.paused);
        assert_eq!(add.headers.len(), 1);
        assert_eq!(add.headers[0].name, "Authorization");
        assert_eq!(add.headers[0].value, "Bearer xyz");
    }

    #[test]
    fn header_arg_rejects_missing_colon() {
        assert!("no-colon-here".parse::<HeaderArg>().is_err());
        let h: HeaderArg = "X-Test:  value ".parse().unwrap();
        assert_eq!(h.name, "X-Test");
        assert_eq!(h.value, "value");
    }

    #[test]
    fn list_accepts_repeated_state_flags() {
        let cli = parse(&["list", "--state", "downloading", "--state", "paused", "--limit", "10"]);
        let Some(Commands::List(list)) = cli.command else {
            panic!("expected List");
        };
        assert_eq!(list.state, vec!["downloading", "paused"]);
        assert_eq!(list.limit, Some(10));
    }

    #[test]
    fn ids_args_require_at_least_one_id() {
        assert!(Cli::try_parse_from(["osprey", "pause"]).is_err());
        let cli = parse(&["pause", "abc123", "def456"]);
        assert!(matches!(cli.command, Some(Commands::Pause(IdsArgs { ids })) if ids == vec!["abc123", "def456"]));
    }

    #[test]
    fn mode_rejects_unknown_value() {
        assert!(Cli::try_parse_from(["osprey", "mode", "turbo"]).is_err());
        let cli = parse(&["mode", "browsing"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Mode(ModeArgs { mode: TrafficModeArg::Browsing }))
        ));
    }

    #[test]
    fn queue_create_parses_nested_subcommand() {
        let cli = parse(&["queue", "create", "Nightly", "--max", "3"]);
        let Some(Commands::Queue(q)) = cli.command else {
            panic!("expected Queue");
        };
        match q.command {
            QueueCommand::Create(c) => {
                assert_eq!(c.name, "Nightly");
                assert_eq!(c.max, 3);
            }
            other => panic!("expected Create, got {other:?}"),
        }
    }

    #[test]
    fn headless_flag_works_without_a_subcommand() {
        let cli = parse(&["--headless"]);
        assert!(cli.headless);
        assert!(cli.command.is_none());
    }
}
