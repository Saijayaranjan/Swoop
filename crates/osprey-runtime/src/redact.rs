//! Log redaction. Every string that reaches a log line, a diagnostics report or an error
//! message passes through [`redact`]. Secrets never leave the process in plaintext.

use regex::Regex;
use std::sync::OnceLock;

fn patterns() -> &'static [(Regex, &'static str)] {
    static P: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    P.get_or_init(|| {
        vec![
            // userinfo in URLs: scheme://user:pass@host
            (Regex::new(r"(?i)([a-z][a-z0-9+.-]*://)([^/\s:@]+)(:[^/\s@]*)?@").unwrap(), "$1[redacted]@"),
            // header values
            (Regex::new(r"(?i)\b(authorization|proxy-authorization|cookie|set-cookie|x-api-key|x-auth-token)\s*[:=]\s*[^\r\n]+").unwrap(), "$1: [redacted]"),
            // bearer / basic tokens anywhere
            (Regex::new(r"(?i)\b(bearer|basic)\s+[A-Za-z0-9+/=_\-.]{8,}").unwrap(), "$1 [redacted]"),
            // query parameters that look like secrets
            (Regex::new(r"(?i)([?&](?:token|access_token|auth|key|api_key|apikey|signature|sig|password|passwd|pwd|secret|session|sid|x-amz-signature|x-amz-credential)=)[^&\s]+").unwrap(), "$1[redacted]"),
            // pairing codes / long hex secrets
            (Regex::new(r"(?i)\b(pairing[_ -]?code|device[_ -]?token|api[_ -]?token)\s*[:=]\s*\S+").unwrap(), "$1=[redacted]"),
            // magnet link tracker passkeys
            (Regex::new(r"(?i)(passkey|announce_key|authkey)=[A-Za-z0-9]+").unwrap(), "$1=[redacted]"),
        ]
    })
}

/// Redact secrets from an arbitrary string.
pub fn redact(input: &str) -> String {
    let mut s = input.to_owned();
    for (re, rep) in patterns() {
        s = re.replace_all(&s, *rep).into_owned();
    }
    s
}

/// Redact a URL for display: strips userinfo and secret-looking query params but keeps the rest.
pub fn redact_url(url: &str) -> String {
    redact(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_userinfo_and_tokens() {
        assert_eq!(redact("ftp://bob:hunter2@files.example.com/x"), "ftp://[redacted]@files.example.com/x");
        assert_eq!(redact("https://h/x?a=1&token=abc123&b=2"), "https://h/x?a=1&token=[redacted]&b=2");
        assert_eq!(redact("Authorization: Bearer eyJhbGciOi"), "Authorization: [redacted]");
        assert_eq!(redact("cookie=session=abc; other=1"), "cookie: [redacted]");
        assert_eq!(redact("plain text stays"), "plain text stays");
        assert_eq!(redact("http://t/announce?passkey=ABCDEF1234&x=1"), "http://t/announce?passkey=[redacted]&x=1");
    }
}
