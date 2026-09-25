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
    /// The body was an HTML page (login/captcha/interstitial) instead of the expected file.
    UnexpectedContent,
    /// HLS playlist without `#EXT-X-ENDLIST` (live stream) — not supported.
    LiveStreamUnsupported,
    /// Server-side quota / bandwidth cap reached.
    QuotaExceeded,
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
    /// Continue with reduced capability (single connection, no resume).
    Degrade,
    /// Pause and wait for an external condition (disk space, volume, network) to change.
    WaitForCondition,
}

impl ErrorKind {
    pub fn class(self) -> FailureClass {
        use ErrorKind::*;
        match self {
            DnsFailure | ConnectionTimeout | ConnectionReset | ReadTimeout | Truncated
            | ServerError | NoPeers | DhtUnavailable | TrackerFailure | TlsFailure => {
                FailureClass::Transient
            }
            Throttled | QuotaExceeded => FailureClass::Throttled,
            ConnectionRefused | ExpiredUrl | MirrorExhausted | RedirectLoop | Forbidden
            | UnexpectedContent => FailureClass::SourceProblem,
            AuthenticationRequired
            | DiskWriteError
            | DiskReadError
            | PermissionDenied
            | FileExists
            | CertificateInvalid
            | ProxyError => FailureClass::NeedsUser,
            DiskFull | VolumeUnavailable | NetworkUnavailable => FailureClass::WaitForCondition,
            RangeNotSupported => FailureClass::Degrade,
            InvalidUrl
            | UnsupportedScheme
            | NotFound
            | InvalidFilename
            | PathTraversal
            | InvalidTorrent
            | ParseError
            | ProtectedContent
            | LiveStreamUnsupported
            | Cancelled
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
            UnexpectedContent => "error.unexpected_content",
            LiveStreamUnsupported => "error.live_stream",
            QuotaExceeded => "error.quota",
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
    /// Server-suggested delay (`Retry-After`) that overrides the backoff policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    pub at: crate::Millis,
}

/// Where an I/O error came from; the same `std::io::ErrorKind` means different things on a
/// socket and on a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IoContext {
    Disk,
    Network,
}

impl TaskError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            status_code: None,
            detail: None,
            source_url: None,
            retry_after_ms: None,
            at: crate::Millis::now(),
        }
    }
    pub fn with_retry_after_ms(mut self, ms: u64) -> Self {
        self.retry_after_ms = Some(ms);
        self
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

    /// Classify an HTTP status code. 403 on a URL that carries signature/expiry parameters is
    /// treated as an expired link (S3/CloudFront behaviour) rather than a hard forbidden.
    pub fn from_http_status(status: u16, url: &str) -> Self {
        let kind = match status {
            401 | 407 => ErrorKind::AuthenticationRequired,
            403 => {
                if url_looks_signed(url) {
                    ErrorKind::ExpiredUrl
                } else {
                    ErrorKind::Forbidden
                }
            }
            404 | 410 => ErrorKind::NotFound,
            // 416 on a resume means the resource changed size (or is already complete — the
            // engine checks `committed == total` before reporting this).
            416 => ErrorKind::SourceChanged,
            429 => ErrorKind::Throttled,
            509 => ErrorKind::QuotaExceeded,
            500..=599 => ErrorKind::ServerError,
            _ => ErrorKind::Unknown,
        };
        Self::new(kind, format!("HTTP {status}"))
            .with_status(status)
            .with_source(url.to_owned())
    }

    /// Classify a std::io error given where it happened.
    pub fn from_io_ctx(err: &std::io::Error, context: &str, io: IoContext) -> Self {
        use std::io::ErrorKind as IoKind;
        let kind = match (err.kind(), io) {
            (IoKind::PermissionDenied, IoContext::Disk) => ErrorKind::PermissionDenied,
            (IoKind::NotFound, IoContext::Disk) => ErrorKind::VolumeUnavailable,
            (IoKind::AlreadyExists, IoContext::Disk) => ErrorKind::FileExists,
            (IoKind::TimedOut, _) => ErrorKind::ConnectionTimeout,
            (IoKind::ConnectionRefused, _) => ErrorKind::ConnectionRefused,
            (
                IoKind::ConnectionReset
                | IoKind::ConnectionAborted
                | IoKind::BrokenPipe
                | IoKind::NotConnected,
                _,
            ) => ErrorKind::ConnectionReset,
            (IoKind::UnexpectedEof, _) => ErrorKind::Truncated,
            (IoKind::Interrupted, _) => ErrorKind::Cancelled,
            (_, IoContext::Disk) => match err.raw_os_error() {
                Some(28) => ErrorKind::DiskFull,  // ENOSPC
                Some(122) => ErrorKind::DiskFull, // EDQUOT (Linux)
                Some(69) if cfg!(target_os = "macos") => ErrorKind::DiskFull, // EDQUOT (macOS)
                Some(30) => ErrorKind::PermissionDenied, // EROFS
                Some(63) | Some(36) => ErrorKind::InvalidFilename, // ENAMETOOLONG
                _ => ErrorKind::DiskWriteError,
            },
            (_, IoContext::Network) => ErrorKind::ConnectionReset,
        };
        Self::new(kind, format!("{context}: {err}"))
    }

    /// Classify a std::io error from a disk operation (the common case).
    pub fn from_io(err: &std::io::Error, context: &str) -> Self {
        Self::from_io_ctx(err, context, IoContext::Disk)
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

/// Does the URL look like a pre-signed/expiring link?
pub fn url_looks_signed(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let q = match lower.split_once('?') {
        Some((_, q)) => q,
        None => return false,
    };
    [
        "x-amz-signature",
        "x-amz-expires",
        "x-goog-signature",
        "signature=",
        "sig=",
        "expires=",
        "expiry=",
        "token=",
        "x-amz-credential",
        "policy=",
    ]
    .iter()
    .any(|k| q.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_status_classification() {
        assert_eq!(
            TaskError::from_http_status(404, "u").kind,
            ErrorKind::NotFound
        );
        assert_eq!(
            TaskError::from_http_status(429, "u").class(),
            FailureClass::Throttled
        );
        assert_eq!(
            TaskError::from_http_status(503, "u").class(),
            FailureClass::Transient
        );
        assert_eq!(
            TaskError::from_http_status(401, "u").class(),
            FailureClass::NeedsUser
        );
        assert!(!TaskError::from_http_status(404, "u").is_retryable());
        assert_eq!(
            TaskError::from_http_status(403, "https://b.s3.amazonaws.com/k?X-Amz-Signature=abc")
                .kind,
            ErrorKind::ExpiredUrl
        );
        assert_eq!(
            TaskError::from_http_status(403, "https://x/y").kind,
            ErrorKind::Forbidden
        );
        assert_eq!(
            TaskError::from_http_status(416, "u").class(),
            FailureClass::RestartFromScratch
        );
        assert_eq!(ErrorKind::RangeNotSupported.class(), FailureClass::Degrade);
        assert_eq!(ErrorKind::DiskFull.class(), FailureClass::WaitForCondition);
        assert_eq!(ErrorKind::TlsFailure.class(), FailureClass::Transient);
    }

    #[test]
    fn enospc_is_disk_full() {
        let e = std::io::Error::from_raw_os_error(28);
        assert_eq!(TaskError::from_io(&e, "write").kind, ErrorKind::DiskFull);
        let other = std::io::Error::other("weird");
        assert_eq!(
            TaskError::from_io_ctx(&other, "read", IoContext::Network).kind,
            ErrorKind::ConnectionReset
        );
        assert_eq!(
            TaskError::from_io_ctx(&other, "write", IoContext::Disk).kind,
            ErrorKind::DiskWriteError
        );
    }
}
