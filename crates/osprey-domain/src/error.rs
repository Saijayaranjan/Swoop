//! Error taxonomy. Every failure is classified so the recovery strategy is chosen from data,
//! and so the UI can explain *why* a download failed in plain language.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Fine-grained failure kind. Keep this list flat: it is what the diagnostics UI, the
/// health score and the smart-recovery policy key on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    InvalidUrl,
    UnsupportedScheme,
    DnsFailure,
    TlsFailure,
    CertificateInvalid,
    ConnectionRefused,
    ConnectionTimeout,
    ConnectionReset,
    ReadTimeout,
    ProxyError,
    /// HTTP 401 / FTP 530 / tracker auth.
    AuthenticationRequired,
    /// HTTP 403.
    Forbidden,
    /// HTTP 404 / 410.
    NotFound,
    /// HTTP 429 or a heuristically detected throttle.
    Throttled,
    /// HTTP 5xx.
    ServerError,
    /// A server that ignores `Range` cannot be segmented or resumed.
    RangeNotSupported,
    /// The resource changed between sessions (ETag / Last-Modified / size mismatch).
    SourceChanged,
    /// Pre-signed URL expired.
    ExpiredUrl,
    /// Redirect loop or too many redirects.
    RedirectLoop,
    /// Unexpected end of body.
    Truncated,
    ChecksumMismatch,
    DiskFull,
    DiskWriteError,
    DiskReadError,
    PermissionDenied,
    /// Destination volume is gone (ejected drive).
    VolumeUnavailable,
    InvalidFilename,
    PathTraversal,
    FileExists,
    InvalidTorrent,
    TrackerFailure,
    NoPeers,
    DhtUnavailable,
    /// Playlist / metalink / HTML parse failure.
    ParseError,
    /// DRM or encryption we refuse to touch.
    ProtectedContent,
    MirrorExhausted,
    NetworkUnavailable,
    Cancelled,
    /// Internal invariant broken; always a bug.
    Internal,
    Unknown,
}

/// The recovery policy classifies every error into one of these buckets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    /// Retry with exponential backoff, same settings.
    Transient,
    /// Retry with fewer connections and a longer backoff.
    Throttled,
    /// Retry only after switching mirror / re-resolving the source.
    SourceProblem,
    /// Do not retry automatically; the user must fix something (auth, disk, path).
    NeedsUser,
    /// Never retry; the resource is gone or the request is invalid.
    Permanent,
    /// Discard partial data and start again (checksum mismatch, source changed).
    RestartFromScratch,
}

impl ErrorKind {
    pub fn class(self) -> FailureClass {
        use ErrorKind::*;
        match self {
            DnsFailure | ConnectionTimeout | ConnectionReset | ReadTimeout | Truncated
            | ServerError | NetworkUnavailable | NoPeers | DhtUnavailable | TrackerFailure => {
                FailureClass::Transient
            }
            Throttled => FailureClass::Throttled,
            ConnectionRefused | ExpiredUrl | MirrorExhausted | RedirectLoop | ProxyError => {
                FailureClass::SourceProblem
            }
            AuthenticationRequired | Forbidden | DiskFull | DiskWriteError | DiskReadError
            | PermissionDenied | VolumeUnavailable | FileExists | CertificateInvalid
            | TlsFailure => FailureClass::NeedsUser,
            InvalidUrl | UnsupportedScheme | NotFound | RangeNotSupported | InvalidFilename
            | PathTraversal | InvalidTorrent | ParseError | ProtectedContent | Cancelled
            | Internal => FailureClass::Permanent,
            ChecksumMismatch | SourceChanged => FailureClass::RestartFromScratch,
            Unknown => FailureClass::Transient,
        }
    }

    pub fn is_retryable(self) -> bool {
        matches!(
            self.class(),
            FailureClass::Transient | FailureClass::Throttled | FailureClass::SourceProblem
        )
    }

    /// Localisation key for the human explanation.
    pub fn explanation_key(self) -> &'static str {
        use ErrorKind::*;
        match self {
            InvalidUrl => "error.invalid_url",
            UnsupportedScheme => "error.unsupported_scheme",
            DnsFailure => "error.dns",
            TlsFailure => "error.tls",
            CertificateInvalid => "error.certificate",
            ConnectionRefused => "error.refused",
            ConnectionTimeout => "error.connect_timeout",
            ConnectionReset => "error.reset",
            ReadTimeout => "error.read_timeout",
            ProxyError => "error.proxy",
            AuthenticationRequired => "error.auth_required",
            Forbidden => "error.forbidden",
            NotFound => "error.not_found",
            Throttled => "error.throttled",
            ServerError => "error.server",
            RangeNotSupported => "error.range_not_supported",
            SourceChanged => "error.source_changed",
            ExpiredUrl => "error.expired_url",
            RedirectLoop => "error.redirect_loop",
            Truncated => "error.truncated",
            ChecksumMismatch => "error.checksum",
            DiskFull => "error.disk_full",
            DiskWriteError => "error.disk_write",
            DiskReadError => "error.disk_read",
            PermissionDenied => "error.permission",
            VolumeUnavailable => "error.volume",
            InvalidFilename => "error.filename",
            PathTraversal => "error.path_traversal",
            FileExists => "error.file_exists",
            InvalidTorrent => "error.invalid_torrent",
            TrackerFailure => "error.tracker",
            NoPeers => "error.no_peers",
            DhtUnavailable => "error.dht",
            ParseError => "error.parse",
            ProtectedContent => "error.protected",
            MirrorExhausted => "error.mirrors_exhausted",
            NetworkUnavailable => "error.network_unavailable",
            Cancelled => "error.cancelled",
            Internal => "error.internal",
            Unknown => "error.unknown",
        }
    }
}

/// A classified, serialisable error attached to a task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{kind:?}: {message}")]
pub struct TaskError {
    pub kind: ErrorKind,
    /// Short, already-redacted human message (English fallback; UI localises by `kind`).
    pub message: String,
    /// HTTP status or FTP reply code, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
    /// Technical detail for "Copy diagnostics" (already redacted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Which source URL / mirror produced it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    pub at: crate::Millis,
}

impl TaskError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            status_code: None,
            detail: None,
            source_url: None,
            at: crate::Millis::now(),
        }
    }
    pub fn with_status(mut self, status: u16) -> Self {
        self.status_code = Some(status);
        self
    }
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source_url = Some(source.into());
        self
    }
    pub fn class(&self) -> FailureClass {
        self.kind.class()
    }
    pub fn is_retryable(&self) -> bool {
        self.kind.is_retryable()
    }
    pub fn cancelled() -> Self {
        Self::new(ErrorKind::Cancelled, "cancelled")
    }
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::Internal, msg)
    }

    /// Classify an HTTP status code.
    pub fn from_http_status(status: u16, url: &str) -> Self {
        let kind = match status {
            401 | 407 => ErrorKind::AuthenticationRequired,
            403 => ErrorKind::Forbidden,
            404 | 410 => ErrorKind::NotFound,
            416 => ErrorKind::RangeNotSupported,
            429 => ErrorKind::Throttled,
            500..=599 => ErrorKind::ServerError,
            _ => ErrorKind::Unknown,
        };
        Self::new(kind, format!("HTTP {status}"))
            .with_status(status)
            .with_source(url.to_owned())
    }

    /// Classify a std::io error.
    pub fn from_io(err: &std::io::Error, context: &str) -> Self {
        use std::io::ErrorKind as IoKind;
        let kind = match err.kind() {
            IoKind::PermissionDenied => ErrorKind::PermissionDenied,
            IoKind::NotFound => ErrorKind::VolumeUnavailable,
            IoKind::AlreadyExists => ErrorKind::FileExists,
            IoKind::TimedOut => ErrorKind::ConnectionTimeout,
            IoKind::ConnectionRefused => ErrorKind::ConnectionRefused,
            IoKind::ConnectionReset | IoKind::ConnectionAborted | IoKind::BrokenPipe => {
                ErrorKind::ConnectionReset
            }
            IoKind::UnexpectedEof => ErrorKind::Truncated,
            IoKind::Interrupted => ErrorKind::Cancelled,
            _ => {
                if err.raw_os_error() == Some(28) {
                    ErrorKind::DiskFull // ENOSPC
                } else {
                    ErrorKind::DiskWriteError
                }
            }
        };
        Self::new(kind, format!("{context}: {err}"))
    }
}

/// Errors raised by the services/API layer (not attached to a task).
#[derive(Debug, thiserror::Error, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "type", content = "message")]
pub enum DomainError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid transition: {0}")]
    InvalidTransition(String),
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("engine error: {0}")]
    Engine(String),
    #[error("unavailable: {0}")]
    Unavailable(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl DomainError {
    pub fn not_found(what: impl fmt::Display) -> Self {
        Self::NotFound(what.to_string())
    }
    pub fn validation(what: impl fmt::Display) -> Self {
        Self::Validation(what.to_string())
    }
    pub fn internal(what: impl fmt::Display) -> Self {
        Self::Internal(what.to_string())
    }
}

pub type DomainResult<T> = Result<T, DomainError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_status_classification() {
        assert_eq!(TaskError::from_http_status(404, "u").kind, ErrorKind::NotFound);
        assert_eq!(TaskError::from_http_status(429, "u").class(), FailureClass::Throttled);
        assert_eq!(TaskError::from_http_status(503, "u").class(), FailureClass::Transient);
        assert_eq!(TaskError::from_http_status(401, "u").class(), FailureClass::NeedsUser);
        assert!(!TaskError::from_http_status(404, "u").is_retryable());
    }

    #[test]
    fn enospc_is_disk_full() {
        let e = std::io::Error::from_raw_os_error(28);
        assert_eq!(TaskError::from_io(&e, "write").kind, ErrorKind::DiskFull);
    }
}
