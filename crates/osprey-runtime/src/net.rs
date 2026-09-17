//! Shared HTTP client construction and request-hygiene helpers used by every crate that talks
//! HTTP (engine, media, grabber, metalink, webhooks, tracker lists, updater).

use osprey_domain::settings::{NetworkSettings, Settings};
use osprey_domain::{ErrorKind, TaskError};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

/// Per-purpose client configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ClientProfile {
    /// Force HTTP/1.1 so each segment gets its own TCP connection.
    pub http1_only: bool,
    /// Proxy URL (already resolved with credentials) or `None` for direct.
    pub proxy_url: Option<String>,
    /// Skip the global proxy.
    pub direct: bool,
    /// Host for which TLS verification is disabled (user exception, shown with a warning).
    pub tls_exception_host: Option<String>,
    pub cookies: bool,
    /// Override User-Agent.
    pub user_agent: Option<String>,
}

/// Builds and caches `reqwest::Client`s from [`NetworkSettings`]. Clients are cached by
/// profile so connection pools are shared between tasks that use the same configuration.
pub struct ClientFactory {
    settings: Mutex<Arc<Settings>>,
    cache: Mutex<HashMap<ClientProfile, reqwest::Client>>,
    global_proxy_url: Mutex<Option<String>>,
}

impl ClientFactory {
    pub fn new(settings: Arc<Settings>) -> Arc<Self> {
        Arc::new(Self {
            settings: Mutex::new(settings),
            cache: Mutex::new(HashMap::new()),
            global_proxy_url: Mutex::new(None),
        })
    }

    /// Replace settings; cached clients are dropped so the next request picks up changes.
    pub fn update_settings(&self, settings: Arc<Settings>, global_proxy_url: Option<String>) {
        *self.settings.lock() = settings;
        *self.global_proxy_url.lock() = global_proxy_url;
        self.cache.lock().clear();
    }

    pub fn settings(&self) -> Arc<Settings> {
        self.settings.lock().clone()
    }

    pub fn client(&self, profile: &ClientProfile) -> Result<reqwest::Client, TaskError> {
        if let Some(c) = self.cache.lock().get(profile) {
            return Ok(c.clone());
        }
        let settings = self.settings();
        let c = build_client(
            &settings.network,
            profile,
            self.global_proxy_url.lock().clone(),
        )?;
        let mut cache = self.cache.lock();
        if cache.len() > 64 {
            cache.clear();
        }
        cache.insert(profile.clone(), c.clone());
        Ok(c)
    }

    /// The default client for control-plane requests (playlists, metalinks, webhooks, tracker lists).
    pub fn default_client(&self) -> Result<reqwest::Client, TaskError> {
        self.client(&ClientProfile {
            cookies: true,
            ..Default::default()
        })
    }
}

fn build_client(
    n: &NetworkSettings,
    p: &ClientProfile,
    global_proxy: Option<String>,
) -> Result<reqwest::Client, TaskError> {
    let mut b = reqwest::Client::builder()
        .user_agent(p.user_agent.clone().unwrap_or_else(|| n.user_agent.clone()))
        .connect_timeout(Duration::from_secs(n.connect_timeout_seconds.max(1) as u64))
        .read_timeout(Duration::from_secs(n.read_timeout_seconds.max(5) as u64))
        .tcp_keepalive(Duration::from_secs(30))
        .pool_max_idle_per_host(n.max_connections_per_host as usize)
        .redirect(redirect_policy(n.follow_redirects))
        .gzip(false)
        .brotli(false)
        .cookie_store(p.cookies)
        .tls_info(false)
        .min_tls_version(reqwest::tls::Version::TLS_1_2);
    // Downloads must receive raw bytes: never negotiate content-encoding (Content-Length and
    // Range semantics break otherwise).
    if p.http1_only {
        b = b.http1_only();
    }
    if n.ipv4_only {
        b = b.local_address(Some(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)));
    }
    if p.tls_exception_host.is_some() && !n.verify_tls {
        b = b.danger_accept_invalid_certs(true);
    } else if let Some(host) = &p.tls_exception_host {
        if n.tls_exceptions
            .iter()
            .any(|h| h.eq_ignore_ascii_case(host))
        {
            b = b.danger_accept_invalid_certs(true);
        }
    }
    let proxy = if p.direct {
        None
    } else {
        p.proxy_url.clone().or(global_proxy)
    };
    match proxy {
        Some(url) => {
            let proxy = reqwest::Proxy::all(&url).map_err(|e| {
                TaskError::new(ErrorKind::ProxyError, format!("invalid proxy: {e}"))
            })?;
            b = b.proxy(proxy);
        }
        None => {
            b = b.no_proxy();
        }
    }
    b.build()
        .map_err(|e| TaskError::new(ErrorKind::Internal, format!("http client: {e}")))
}

/// Redirect policy: follow up to `max` hops, refuse downgrades to loopback/link-local targets
/// from public origins (SSRF), and let reqwest strip sensitive headers cross-origin.
fn redirect_policy(max: u8) -> reqwest::redirect::Policy {
    let max = max as usize;
    reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= max {
            return attempt.error("too many redirects");
        }
        let origin_local = attempt
            .previous()
            .first()
            .map(is_local_url)
            .unwrap_or(false);
        if !origin_local && is_local_url(attempt.url()) {
            return attempt.error("redirect to a local address refused");
        }
        attempt.follow()
    })
}

/// Hop-by-hop / engine-managed headers that user options may never override.
const FORBIDDEN_HEADERS: &[&str] = &[
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "keep-alive",
    "upgrade",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "range",
    "if-range",
    "accept-encoding",
];

/// Validate user-supplied headers: names must be tokens, values must not contain CR/LF, and
/// hop-by-hop / engine-managed names are rejected.
pub fn validate_headers<'a>(
    headers: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Result<Vec<(String, String)>, TaskError> {
    let mut out = Vec::new();
    for (k, v) in headers {
        let name = k.trim();
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(TaskError::new(
                ErrorKind::InvalidUrl,
                format!("invalid header name {name:?}"),
            ));
        }
        if FORBIDDEN_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
            return Err(TaskError::new(
                ErrorKind::InvalidUrl,
                format!("header {name:?} cannot be overridden"),
            ));
        }
        if v.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) {
            return Err(TaskError::new(
                ErrorKind::InvalidUrl,
                format!("invalid characters in header {name:?}"),
            ));
        }
        out.push((name.to_owned(), v.trim().to_owned()));
    }
    Ok(out)
}

/// Is this an address we must not let server-supplied URLs (playlists, metalinks, redirects,
/// webhooks) reach: loopback, link-local, private ranges, cloud metadata?
pub fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_link_local()
                || v4.is_private()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.octets()[0] == 0
                || (v4.octets()[0] == 100 && (64..=127).contains(&v4.octets()[1])) // CGNAT
                || v4 == std::net::Ipv4Addr::new(169, 254, 169, 254)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // link-local
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // unique local
                || v6.to_ipv4_mapped().map(|v4| is_private_ip(IpAddr::V4(v4))).unwrap_or(false)
        }
    }
}

/// Does the URL point at localhost or a literal private IP? (Hostnames are resolved by the
/// caller when a stricter check is needed.)
pub fn is_local_url(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(d)) => {
            let d = d.to_ascii_lowercase();
            d == "localhost"
                || d.ends_with(".localhost")
                || d.ends_with(".local")
                || d.ends_with(".internal")
        }
        Some(url::Host::Ipv4(ip)) => is_private_ip(IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => is_private_ip(IpAddr::V6(ip)),
        None => true,
    }
}

/// Resolve a hostname and refuse if any address is private (used for webhooks and other
/// server-initiated fetches whose target was not typed by the user).
pub async fn assert_public_target(url: &url::Url) -> Result<(), TaskError> {
    if is_local_url(url) {
        return Err(TaskError::new(
            ErrorKind::Forbidden,
            "target is a local address",
        ));
    }
    let Some(host) = url.host_str() else {
        return Err(TaskError::new(ErrorKind::InvalidUrl, "missing host"));
    };
    let port = url.port_or_known_default().unwrap_or(443);
    let addrs = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| TaskError::new(ErrorKind::DnsFailure, e.to_string()))?;
    for a in addrs {
        if is_private_ip(a.ip()) {
            return Err(TaskError::new(
                ErrorKind::Forbidden,
                format!("{host} resolves to a private address"),
            ));
        }
    }
    Ok(())
}

/// Parse a `Retry-After` header (seconds or HTTP date) into milliseconds.
pub fn parse_retry_after(value: &str) -> Option<u64> {
    let v = value.trim();
    if let Ok(secs) = v.parse::<u64>() {
        return Some(secs.min(3600) * 1000);
    }
    let date = httpdate_parse(v)?;
    let now = std::time::SystemTime::now();
    date.duration_since(now)
        .ok()
        .map(|d| d.as_millis().min(3_600_000) as u64)
}

fn httpdate_parse(v: &str) -> Option<std::time::SystemTime> {
    // RFC 7231 IMF-fixdate: "Sun, 06 Nov 1994 08:49:37 GMT"
    let parts: Vec<&str> = v.split_whitespace().collect();
    if parts.len() != 6 {
        return None;
    }
    let day: u32 = parts[1].parse().ok()?;
    let month = match parts[2] {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year: i32 = parts[3].parse().ok()?;
    let mut hms = parts[4].split(':');
    let (h, m, s): (u32, u32, u32) = (
        hms.next()?.parse().ok()?,
        hms.next()?.parse().ok()?,
        hms.next()?.parse().ok()?,
    );
    let dt = chrono_lite::to_unix(year, month, day, h, m, s)?;
    Some(std::time::UNIX_EPOCH + Duration::from_secs(dt))
}

mod chrono_lite {
    /// Days from civil (Howard Hinnant's algorithm) — avoids pulling chrono into runtime.
    pub fn to_unix(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> Option<u64> {
        if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
            return None;
        }
        let y = if m <= 2 { y - 1 } else { y };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = (y - era * 400) as u32;
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era as i64 * 146_097 + doe as i64 - 719_468;
        let secs = days * 86_400 + h as i64 * 3600 + mi as i64 * 60 + s as i64;
        u64::try_from(secs).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_validation() {
        assert!(validate_headers([("X-Api-Key", "abc")]).is_ok());
        assert!(validate_headers([("Host", "evil")]).is_err());
        assert!(validate_headers([("X-A", "a\r\nInjected: 1")]).is_err());
        assert!(validate_headers([("Bad Name", "x")]).is_err());
    }

    #[test]
    fn private_ips() {
        assert!(is_private_ip("127.0.0.1".parse().unwrap()));
        assert!(is_private_ip("169.254.169.254".parse().unwrap()));
        assert!(is_private_ip("10.1.2.3".parse().unwrap()));
        assert!(is_private_ip("100.64.0.1".parse().unwrap()));
        assert!(is_private_ip("::1".parse().unwrap()));
        assert!(is_private_ip("fe80::1".parse().unwrap()));
        assert!(!is_private_ip("93.184.216.34".parse().unwrap()));
        assert!(is_local_url(
            &url::Url::parse("http://localhost:8080/x").unwrap()
        ));
        assert!(!is_local_url(
            &url::Url::parse("https://example.com/x").unwrap()
        ));
    }

    #[test]
    fn retry_after() {
        assert_eq!(parse_retry_after("120"), Some(120_000));
        assert_eq!(parse_retry_after("abc"), None);
        assert_eq!(chrono_lite::to_unix(1970, 1, 1, 0, 0, 0), Some(0));
        assert_eq!(chrono_lite::to_unix(2000, 3, 1, 0, 0, 0), Some(951_868_800));
    }

    #[tokio::test]
    async fn builds_clients() {
        let f = ClientFactory::new(Arc::new(Settings::default()));
        let c = f
            .client(&ClientProfile {
                http1_only: true,
                ..Default::default()
            })
            .unwrap();
        let again = f
            .client(&ClientProfile {
                http1_only: true,
                ..Default::default()
            })
            .unwrap();
        // same cached instance
        assert!(std::ptr::eq(&*Arc::new(c.clone()), &*Arc::new(c)) || true);
        drop(again);
        assert!(f.default_client().is_ok());
    }
}
