//! `swoop update`: check GitHub Releases for a newer Swoop (`check`), plus hidden release-tooling
//! and installer subcommands (`keygen`, `sign`, `verify`, `apply`).
//!
//! The CLI and headless servers only ever *report* updates; installing is the macOS app's job,
//! which runs `swoop update apply` (from its bundled copy of this binary) to swap bundles after
//! it has quit.

use std::path::PathBuf;

use clap::Subcommand;
use swoop_update::{CheckerOptions, UpdateChecker, UpdateStatus};

use crate::error::{CliError, CliResult};
use crate::output::print_json;

#[derive(clap::Args, Debug)]
pub struct UpdateArgs {
    #[command(subcommand)]
    pub command: UpdateCommand,
}

#[derive(Subcommand, Debug)]
pub enum UpdateCommand {
    /// Check GitHub Releases for a newer version (never installs anything).
    Check {
        /// Include prereleases (the beta channel).
        #[arg(long)]
        beta: bool,
    },
    /// Create a release signing key (private key file, chmod 600) and print its public key.
    #[command(hide = true)]
    Keygen {
        #[arg(long, value_name = "PATH")]
        out: PathBuf,
    },
    /// Sign a release DMG: writes `<file>.sig` (base64 Ed25519 over the SHA-256).
    #[command(hide = true)]
    Sign {
        #[arg(long, value_name = "PATH")]
        key: PathBuf,
        file: PathBuf,
        /// Signature output path (default `<file>.sig`).
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
    },
    /// Verify a DMG against its `.sig` with the release key compiled into this binary.
    #[command(hide = true)]
    Verify {
        file: PathBuf,
        /// Signature file (default `<file>.sig`).
        #[arg(long, value_name = "PATH")]
        sig: Option<PathBuf>,
    },
    /// Installer helper launched by the app: wait for it to quit, swap bundles, relaunch.
    #[command(hide = true)]
    Apply {
        #[arg(long)]
        pid: Option<i32>,
        #[arg(long, value_name = "PATH")]
        staged: PathBuf,
        #[arg(long, value_name = "PATH")]
        target: PathBuf,
        #[arg(long)]
        relaunch: bool,
    },
}

pub async fn run(args: UpdateArgs, json: bool) -> CliResult<()> {
    match args.command {
        UpdateCommand::Check { beta } => check(beta, json).await,
        UpdateCommand::Keygen { out } => {
            let pk = swoop_update::signing::generate_signing_key(&out).map_err(other)?;
            if json {
                print_json(&serde_json::json!({ "private_key": out, "public_key": pk }));
            } else {
                println!(
                    "Private key written to {} (keep it secret, back it up).",
                    out.display()
                );
                println!("Public key: {pk}");
            }
            Ok(())
        }
        UpdateCommand::Sign { key, file, out } => {
            let sk = swoop_update::signing::load_signing_key(&key).map_err(other)?;
            let sig = swoop_update::signing::sign_file(&sk, &file)
                .map_err(|e| other(format!("sign {}: {e}", file.display())))?;
            let out = out.unwrap_or_else(|| sig_path(&file));
            std::fs::write(&out, format!("{sig}\n"))
                .map_err(|e| other(format!("write {}: {e}", out.display())))?;
            let pk = swoop_update::signing::public_key_hex(&sk);
            if pk != swoop_update::release_public_key_hex() {
                eprintln!(
                    "warning: this key's public half ({pk}) isn't the release key compiled into \
                     this build; installed apps will refuse the update"
                );
            }
            if json {
                print_json(&serde_json::json!({ "signature": out, "public_key": pk }));
            } else {
                println!("{}", out.display());
            }
            Ok(())
        }
        UpdateCommand::Verify { file, sig } => {
            let sig = sig.unwrap_or_else(|| sig_path(&file));
            let text = std::fs::read_to_string(&sig)
                .map_err(|e| other(format!("read {}: {e}", sig.display())))?;
            let key =
                swoop_update::signing::parse_public_key(swoop_update::release_public_key_hex())
                    .map_err(|e| other(e.message))?;
            swoop_update::signing::verify_file(&key, &file, &text).map_err(|e| other(e.message))?;
            if json {
                print_json(&serde_json::json!({ "verified": true }));
            } else {
                println!("OK: {} is signed with the release key", file.display());
            }
            Ok(())
        }
        UpdateCommand::Apply {
            pid,
            staged,
            target,
            relaunch,
        } => apply(pid, staged, target, relaunch),
    }
}

fn sig_path(file: &std::path::Path) -> PathBuf {
    let mut s = file.as_os_str().to_owned();
    s.push(".sig");
    PathBuf::from(s)
}

fn other(msg: impl std::fmt::Display) -> CliError {
    CliError::Other(anyhow::anyhow!("{msg}"))
}

async fn check(beta: bool, json: bool) -> CliResult<()> {
    let checker = UpdateChecker::new(CheckerOptions::for_version(env!("CARGO_PKG_VERSION")))
        .map_err(|e| other(e.message))?;
    let info = checker.check(beta).await;
    if json {
        print_json(&info);
        return Ok(());
    }
    match info.status {
        UpdateStatus::Available => {
            println!(
                "Swoop {} is available (you have {}).",
                info.latest_version.as_deref().unwrap_or("?"),
                info.current_version
            );
            if let Some(url) = &info.notes_url {
                println!("Release notes: {url}");
            }
            println!("Update from the Swoop app, or download the DMG from the release page.");
        }
        UpdateStatus::UpToDate => println!("Swoop {} is up to date.", info.current_version),
        UpdateStatus::NoReleases => println!(
            "Swoop {} is up to date (no releases have been published yet).",
            info.current_version
        ),
        _ => println!(
            "Couldn't check for updates: {}",
            info.message.as_deref().unwrap_or("unknown error")
        ),
    }
    Ok(())
}

#[cfg(unix)]
fn apply(pid: Option<i32>, staged: PathBuf, target: PathBuf, relaunch: bool) -> CliResult<()> {
    use std::io::Write;
    use swoop_update::install;

    install::detach_session();
    let log_path = directories::BaseDirs::new()
        .map(|b| b.home_dir().join("Library/Logs/Swoop/updater.log"))
        .unwrap_or_else(|| std::env::temp_dir().join("swoop-updater.log"));
    if let Some(dir) = log_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .ok();
    let mut log = |line: &str| {
        let stamp = swoop_domain::Millis::now().to_datetime().to_rfc3339();
        if let Some(f) = file.as_mut() {
            let _ = writeln!(f, "{stamp} {line}");
        }
        eprintln!("{line}");
    };
    let req = install::ApplyRequest::new(pid, staged, target, relaunch);
    install::apply_update(&req, &mut log).map_err(other)
}

#[cfg(not(unix))]
fn apply(_: Option<i32>, _: PathBuf, _: PathBuf, _: bool) -> CliResult<()> {
    Err(other("self-install is only supported by the macOS app"))
}
