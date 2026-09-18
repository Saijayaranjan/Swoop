//! FTP reply codes → error kinds.

use osprey_domain::{ErrorKind, TaskError};

pub fn from_reply(code: u16, text: &str, context: &str) -> TaskError {
    let kind = match code {
        421 | 425 | 426 | 434 | 450 | 451 | 452 => ErrorKind::ConnectionReset,
        430 | 530 | 532 => ErrorKind::AuthenticationRequired,
        550 => {
            let t = text.to_ascii_lowercase();
            if t.contains("permission") || t.contains("denied") || t.contains("access") {
                ErrorKind::Forbidden
            } else {
                ErrorKind::NotFound
            }
        }
        551..=553 => ErrorKind::ServerError,
        500..=504 => ErrorKind::RangeNotSupported,
        _ => ErrorKind::Unknown,
    };
    TaskError::new(
        kind,
        format!(
            "{context}: {code} {}",
            osprey_runtime::redact::redact(text.trim())
        ),
    )
    .with_status(code)
}
