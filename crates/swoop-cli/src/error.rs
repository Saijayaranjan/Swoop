//! The CLI's error type and its mapping to process exit codes.
//!
//! Exit codes: `0` ok, `1` error, `2` usage, `3` the engine is not running, `4` not found.

use std::fmt;

#[derive(Debug)]
pub enum CliError {
    /// Bad arguments / bad input that clap itself didn't catch.
    Usage(String),
    /// Could not reach the engine over the socket or the remote URL.
    NotRunning(String),
    /// The API returned `not_found`, or a local lookup (e.g. a queue name) failed.
    NotFound(String),
    /// The API returned a structured error envelope.
    Api {
        #[allow(dead_code)]
        status: u16,
        kind: String,
        message: String,
    },
    /// Anything else (I/O, parse errors, …).
    Other(anyhow::Error),
}

impl CliError {
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::Usage(_) => 2,
            CliError::NotRunning(_) => 3,
            CliError::NotFound(_) => 4,
            CliError::Api { kind, .. } if kind == "not_found" => 4,
            CliError::Api { .. } => 1,
            CliError::Other(_) => 1,
        }
    }

    pub fn not_running() -> Self {
        CliError::NotRunning(
            "Swoop is not running. Start the app or run `swoop server`.".to_owned(),
        )
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Usage(m) => write!(f, "{m}"),
            CliError::NotRunning(m) => write!(f, "{m}"),
            CliError::NotFound(m) => write!(f, "{m}"),
            CliError::Api { kind, message, .. } => write!(f, "{kind}: {message}"),
            CliError::Other(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CliError {}

impl From<anyhow::Error> for CliError {
    fn from(e: anyhow::Error) -> Self {
        CliError::Other(e)
    }
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError::Other(e.into())
    }
}

impl From<serde_json::Error> for CliError {
    fn from(e: serde_json::Error) -> Self {
        CliError::Other(e.into())
    }
}

pub type CliResult<T> = Result<T, CliError>;
