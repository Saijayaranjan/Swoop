//! The updater's HTTP client: HTTPS only, including every redirect hop.
//!
//! GitHub serves release assets by redirecting `github.com/.../releases/download/...` to
//! `objects.githubusercontent.com`; those hops are followed only while they stay on HTTPS.
//! Debug builds may additionally talk plain HTTP to loopback, for local end-to-end tests.

use std::time::Duration;
use swoop_domain::{ErrorKind, TaskError};
use url::{Host, Url};

const MAX_REDIRECTS: usize = 10;

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        Some(Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}

/// May the updater fetch `url`? HTTPS always; plain HTTP only to loopback and only when
/// `allow_local_http` (debug builds) is set.
pub fn url_allowed(url: &Url, allow_local_http: bool) -> bool {
    match url.scheme() {
        "https" => url.host().is_some(),
        "http" => allow_local_http && is_loopback(url),
        _ => false,
    }
}

/// Same rule for a redirect target.
pub fn redirect_allowed(next: &Url, allow_local_http: bool) -> bool {
    url_allowed(next, allow_local_http)
}

pub fn require_allowed(url: &str, allow_local_http: bool) -> Result<Url, TaskError> {
    let parsed =
        Url::parse(url).map_err(|e| TaskError::new(ErrorKind::InvalidUrl, e.to_string()))?;
    if !url_allowed(&parsed, allow_local_http) {
        return Err(TaskError::new(
            ErrorKind::Forbidden,
            format!("refusing to fetch an update over a non-HTTPS URL: {parsed}"),
        ));
    }
    Ok(parsed)
}

/// Build the client: `User-Agent: Swoop/<version>`, TLS 1.2+, HTTPS-only redirects.
pub fn build_client(
    user_agent: &str,
    allow_local_http: bool,
    proxy_url: Option<&str>,
) -> Result<reqwest::Client, TaskError> {
    let policy = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error("too many redirects");
        }
        if !redirect_allowed(attempt.url(), allow_local_http) {
            let msg = format!("refusing redirect to non-HTTPS URL {}", attempt.url());
            return attempt.error(msg);
        }
        attempt.follow()
    });
    let mut b = reqwest::Client::builder()
        .user_agent(user_agent)
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        .redirect(policy)
        .min_tls_version(reqwest::tls::Version::TLS_1_2);
    if let Some(p) = proxy_url {
        let proxy = reqwest::Proxy::all(p)
            .map_err(|e| TaskError::new(ErrorKind::ProxyError, format!("invalid proxy: {e}")))?;
        b = b.proxy(proxy);
    }
    b.build()
        .map_err(|e| TaskError::new(ErrorKind::Internal, format!("http client: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn only_https_redirects() {
        let gh = u("https://objects.githubusercontent.com/github-production-release-asset/1?sig=x");
        assert!(redirect_allowed(&gh, false));
        assert!(redirect_allowed(
            &u("https://github.com/a/b/releases/download/v1/x.dmg"),
            false
        ));
        for bad in [
            "http://objects.githubusercontent.com/x.dmg",
            "http://127.0.0.1:8080/x.dmg",
            "ftp://example.com/x.dmg",
            "file:///tmp/x.dmg",
            "data:application/octet-stream;base64,AAAA",
        ] {
            assert!(!redirect_allowed(&u(bad), false), "{bad} must be refused");
        }
        // the debug-only loopback allowance never extends to other hosts
        assert!(redirect_allowed(&u("http://127.0.0.1:8080/x.dmg"), true));
        assert!(redirect_allowed(&u("http://localhost:8080/x.dmg"), true));
        assert!(!redirect_allowed(&u("http://example.com/x.dmg"), true));
        assert!(!redirect_allowed(&u("http://10.0.0.1/x.dmg"), true));
    }

    #[tokio::test]
    async fn client_refuses_http_redirect_hop() {
        let server = swoop_testserver::TestServer::start().await;
        // The test server answers `redirect=1` with a 302 to its own plain-HTTP URL.
        let url = server.url("/file/asset.dmg?size=10&redirect=1");
        let strict = build_client("Swoop/test", false, None).unwrap();
        let err = strict.get(&url).send().await.unwrap_err();
        assert!(err.is_redirect(), "expected a redirect error, got {err}");
        // With loopback HTTP allowed (debug builds) the same hop is followed.
        let lax = build_client("Swoop/test", true, None).unwrap();
        assert_eq!(lax.get(&url).send().await.unwrap().status(), 200);
        assert!(require_allowed(&url, false).is_err());
        assert!(require_allowed("https://api.github.com/x", false).is_ok());
    }
}
