//! Mapping of transport failures onto the domain error taxonomy.
//!
//! `reqwest::Error` is opaque by design: the interesting information (DNS vs. TLS vs. reset)
//! lives in the `source()` chain and, for rustls, only in the message text. We walk the chain
//! once, collect every message, and classify from the strongest signal we find. The whole
//! chain (redacted) is kept as `detail` for "Copy diagnostics".

use osprey_domain::{ErrorKind, TaskError};
use osprey_runtime::redact::redact;
use std::error::Error as _;

/// Classify a `reqwest` error that occurred while sending a request or reading a body.
pub fn classify_reqwest(err: &reqwest::Error, url: &str) -> TaskError {
    let (messages, io_kind) = walk_chain(err);
    let lower = messages.to_ascii_lowercase();
    let kind = if err.is_redirect() {
        ErrorKind::RedirectLoop
    } else if err.is_builder() {
        ErrorKind::InvalidUrl
    } else if err.is_connect() {
        classify_connect(err, &lower, io_kind)
    } else if err.is_timeout() {
        // A timeout that is not a connect timeout happened while waiting for bytes.
        ErrorKind::ReadTimeout
    } else if err.is_body() || err.is_decode() {
        classify_body(&lower, io_kind)
    } else if lower.contains("proxy") {
        ErrorKind::ProxyError
    } else if lower.contains("dns") || lower.contains("resolve") {
        ErrorKind::DnsFailure
    } else if is_certificate_text(&lower) {
        ErrorKind::CertificateInvalid
    } else if lower.contains("tls") || lower.contains("handshake") {
        ErrorKind::TlsFailure
    } else {
        classify_body(&lower, io_kind)
    };
    TaskError::new(kind, short_message(kind, &messages))
        .with_source(redact(url))
        .with_detail(redact(&messages))
}

fn classify_connect(
    err: &reqwest::Error,
    lower: &str,
    io_kind: Option<std::io::ErrorKind>,
) -> ErrorKind {
    if err.is_dns()
        || lower.contains("dns")
        || lower.contains("failed to lookup")
        || lower.contains("nodename nor servname")
        || lower.contains("name or service not known")
    {
        return ErrorKind::DnsFailure;
    }
    if is_certificate_text(lower) {
        return ErrorKind::CertificateInvalid;
    }
    if lower.contains("tls") || lower.contains("handshake") || lower.contains("ssl") {
        return ErrorKind::TlsFailure;
    }
    if lower.contains("proxy") {
        return ErrorKind::ProxyError;
    }
    if err.is_timeout() || io_kind == Some(std::io::ErrorKind::TimedOut) {
        return ErrorKind::ConnectionTimeout;
    }
    if lower.contains("refused") || io_kind == Some(std::io::ErrorKind::ConnectionRefused) {
        return ErrorKind::ConnectionRefused;
    }
    if lower.contains("unreachable") || lower.contains("network is down") {
        return ErrorKind::NetworkUnavailable;
    }
    ErrorKind::ConnectionReset
}

fn classify_body(lower: &str, io_kind: Option<std::io::ErrorKind>) -> ErrorKind {
    match io_kind {
        Some(std::io::ErrorKind::UnexpectedEof) => return ErrorKind::Truncated,
        Some(std::io::ErrorKind::TimedOut) => return ErrorKind::ReadTimeout,
        Some(
            std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::NotConnected,
        ) => return ErrorKind::ConnectionReset,
        _ => {}
    }
    if lower.contains("unexpected eof")
        || lower.contains("end of file")
        || lower.contains("incomplete")
        || lower.contains("closed before")
    {
        ErrorKind::Truncated
    } else if lower.contains("timed out") || lower.contains("timeout") {
        ErrorKind::ReadTimeout
    } else {
        ErrorKind::ConnectionReset
    }
}

/// rustls reports certificate problems only through text; these fragments are stable across
/// its releases (`InvalidCertificate(UnknownIssuer)`, `invalid peer certificate`, …).
fn is_certificate_text(lower: &str) -> bool {
    lower.contains("certificate")
        || lower.contains("unknownissuer")
        || lower.contains("notvalidforname")
        || lower.contains("badsignature")
        || lower.contains("expired")
}

/// Collect the messages of the whole source chain and the innermost `io::ErrorKind`, if any.
fn walk_chain(err: &reqwest::Error) -> (String, Option<std::io::ErrorKind>) {
    let mut parts = vec![err.to_string()];
    let mut io_kind = None;
    let mut source = err.source();
    let mut depth = 0;
    while let Some(s) = source {
        if depth > 12 {
            break;
        }
        parts.push(s.to_string());
        if let Some(io) = s.downcast_ref::<std::io::Error>() {
            io_kind = Some(io.kind());
        }
        source = s.source();
        depth += 1;
    }
    (parts.join(": "), io_kind)
}

fn short_message(kind: ErrorKind, chain: &str) -> String {
    let human = match kind {
        ErrorKind::DnsFailure => "could not resolve host",
        ErrorKind::ConnectionTimeout => "connection timed out",
        ErrorKind::ConnectionRefused => "connection refused",
        ErrorKind::ConnectionReset => "connection lost",
        ErrorKind::ReadTimeout => "server stopped sending data",
        ErrorKind::TlsFailure => "TLS handshake failed",
        ErrorKind::CertificateInvalid => "server certificate is not trusted",
        ErrorKind::ProxyError => "proxy connection failed",
        ErrorKind::RedirectLoop => "too many redirects",
        ErrorKind::Truncated => "response ended early",
        ErrorKind::InvalidUrl => "invalid request",
        ErrorKind::NetworkUnavailable => "network unreachable",
        _ => "request failed",
    };
    // Keep the first message fragment so the log line remains actionable without the detail.
    let first = chain.split(": ").next().unwrap_or("");
    if first.is_empty() {
        human.to_owned()
    } else {
        format!("{human} ({})", redact(first))
    }
}

/// Classify an HTTP status returned to a data request, honouring `Retry-After`.
pub fn classify_status(status: u16, url: &str, retry_after: Option<&str>) -> TaskError {
    let mut e = TaskError::from_http_status(status, url).with_source(redact(url));
    if let Some(ms) = retry_after.and_then(osprey_runtime::net::parse_retry_after) {
        e = e.with_retry_after_ms(ms);
    }
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn refused_connection_is_classified() {
        // Port 1 on loopback is closed on every developer machine and CI runner.
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let err = client
            .get("http://127.0.0.1:1/x")
            .send()
            .await
            .expect_err("must fail");
        let e = classify_reqwest(&err, "http://127.0.0.1:1/x");
        assert!(
            matches!(
                e.kind,
                ErrorKind::ConnectionRefused | ErrorKind::ConnectionReset
            ),
            "{e:?}"
        );
        assert_eq!(e.source_url.as_deref(), Some("http://127.0.0.1:1/x"));
        assert!(e.detail.is_some());
    }

    #[tokio::test]
    async fn dns_failure_is_classified() {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let err = client
            .get("http://nonexistent.invalid/x")
            .send()
            .await
            .expect_err("must fail");
        let e = classify_reqwest(&err, "http://nonexistent.invalid/x");
        assert_eq!(e.kind, ErrorKind::DnsFailure, "{e:?}");
    }

    #[test]
    fn status_with_retry_after() {
        let e = classify_status(429, "http://h/x?token=abc", Some("7"));
        assert_eq!(e.kind, ErrorKind::Throttled);
        assert_eq!(e.retry_after_ms, Some(7000));
        assert_eq!(e.source_url.as_deref(), Some("http://h/x?token=[redacted]"));
    }

    #[test]
    fn certificate_text_detection() {
        assert!(is_certificate_text(
            "invalid peer certificate: unknownissuer"
        ));
        assert!(!is_certificate_text("connection reset by peer"));
    }
}
