//! A local HTTP server for tests. Serves deterministic pseudo-random files under `/file/<name>`
//! with behaviours configured per request via query parameters or globally via [`Behaviour`],
//! so engines can be tested against Range, throttling, disconnects, redirects and auth without
//! touching the network.
//!
//! Query parameters understood on `/file/<name>`:
//! * `size=<bytes>`          — file size (default 1 MiB); content is deterministic per name+size
//! * `norange=1`             — ignore `Range`, always 200 with the whole body
//! * `throttle=<bytes/s>`    — pace the body
//! * `fail_after=<bytes>`    — close the connection after sending this many body bytes
//! * `fail_first=<n>`        — first `n` requests to this name answer 503 (counted per server)
//! * `status=<code>`         — always answer this status (e.g. 403, 404, 429)
//! * `redirect=<n>`          — answer 302 to itself with `redirect=n-1` until 0
//! * `auth=user:pass`        — require HTTP Basic
//! * `etag=<tag>`            — ETag to report (default derived from name+size)
//! * `nolength=1`            — chunked transfer without Content-Length
//! * `disposition=<name>`    — Content-Disposition attachment filename
//! * `delay_ms=<n>`          — delay before the response headers
//! * `ctype=<mime>`          — Content-Type
//! * `max_conn=<n>`          — respond 429 while more than n requests for this name are in flight
//! * `corrupt=1`             — flip one byte in the body (checksum mismatch tests)
//!
//! `/html/<page>` serves small HTML pages registered with [`TestServer::add_html`] for crawler tests.
//! `/m3u8/...` serves playlists registered with [`TestServer::add_text`].
//! `/stats` returns request counters as JSON.

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use bytes::Bytes;
use parking_lot::Mutex;
use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;

#[derive(Default)]
struct Inner {
    request_count: AtomicU64,
    per_name_requests: Mutex<HashMap<String, u64>>,
    in_flight: Mutex<HashMap<String, usize>>,
    html: Mutex<HashMap<String, String>>,
    text: Mutex<HashMap<String, (String, String)>>,
    bytes: Mutex<HashMap<String, (Bytes, String)>>,
    /// Requests observed with method, path, range header — for assertions.
    log: Mutex<Vec<RequestRecord>>,
    global_delay: AtomicUsize,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct RequestRecord {
    pub method: String,
    pub path: String,
    pub range: Option<String>,
    pub user_agent: Option<String>,
    pub referer: Option<String>,
    pub authorization: bool,
}

pub struct TestServer {
    pub addr: SocketAddr,
    inner: Arc<Inner>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl TestServer {
    pub async fn start() -> Self {
        let inner = Arc::new(Inner::default());
        let app = Router::new()
            .route("/file/{name}", get(serve_file).head(serve_file))
            .route("/html/{*name}", get(serve_html))
            .route("/robots.txt", get(serve_robots))
            .route("/text/{*name}", get(serve_text))
            .route("/bytes/{*name}", get(serve_bytes).head(serve_bytes))
            .route("/stats", get(stats))
            .with_state(inner.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = rx.await;
                })
                .await
                .ok();
        });
        Self {
            addr,
            inner,
            shutdown: Some(tx),
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    pub fn file_url(&self, name: &str, size: u64) -> String {
        self.url(&format!("/file/{name}?size={size}"))
    }

    pub fn add_html(&self, name: &str, html: &str) {
        self.inner
            .html
            .lock()
            .insert(name.to_owned(), html.to_owned());
    }

    pub fn add_text(&self, path: &str, content_type: &str, body: &str) {
        self.inner
            .text
            .lock()
            .insert(path.to_owned(), (content_type.to_owned(), body.to_owned()));
    }

    pub fn add_bytes(&self, path: &str, content_type: &str, body: Bytes) {
        self.inner
            .bytes
            .lock()
            .insert(path.to_owned(), (body, content_type.to_owned()));
    }

    pub fn request_count(&self) -> u64 {
        self.inner.request_count.load(Ordering::Relaxed)
    }

    pub fn requests(&self) -> Vec<RequestRecord> {
        self.inner.log.lock().clone()
    }

    pub fn requests_for(&self, name: &str) -> u64 {
        *self.inner.per_name_requests.lock().get(name).unwrap_or(&0)
    }

    /// Add a delay (ms) to every response — simulates a slow server for stall tests.
    pub fn set_global_delay_ms(&self, ms: usize) {
        self.inner.global_delay.store(ms, Ordering::Relaxed);
    }

    pub fn stop(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

/// Deterministic content: byte i of file `name` with `size` bytes.
pub fn content_for(name: &str, size: u64) -> Bytes {
    let mut seed = 0x9E3779B97F4A7C15u64;
    for b in name.bytes() {
        seed = seed.wrapping_mul(31).wrapping_add(b as u64);
    }
    seed ^= size;
    let mut out = Vec::with_capacity(size as usize);
    let mut x = seed;
    while out.len() < size as usize {
        // xorshift64*
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        let v = x.wrapping_mul(0x2545F4914F6CDD1D);
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.truncate(size as usize);
    Bytes::from(out)
}

pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(data))
}

fn etag_for(name: &str, size: u64) -> String {
    format!(
        "\"{}-{}\"",
        name.len() * 7919 + name.bytes().map(|b| b as usize).sum::<usize>(),
        size
    )
}

fn parse_range(h: &str, size: u64) -> Option<(u64, u64)> {
    let spec = h.strip_prefix("bytes=")?;
    let (a, b) = spec.split_once('-')?;
    if a.is_empty() {
        let suffix: u64 = b.parse().ok()?;
        if suffix == 0 {
            return None;
        }
        let start = size.saturating_sub(suffix);
        return Some((start, size - 1));
    }
    let start: u64 = a.parse().ok()?;
    let end: u64 = if b.is_empty() {
        size.saturating_sub(1)
    } else {
        b.parse().ok()?
    };
    if start >= size || end < start {
        return None;
    }
    Some((start, end.min(size - 1)))
}

struct InFlightGuard(Arc<Inner>, String);
impl Drop for InFlightGuard {
    fn drop(&mut self) {
        let mut m = self.0.in_flight.lock();
        if let Some(v) = m.get_mut(&self.1) {
            *v = v.saturating_sub(1);
        }
    }
}

async fn serve_file(
    State(inner): State<Arc<Inner>>,
    Path(name): Path<String>,
    Query(q): Query<BTreeMap<String, String>>,
    method: axum::http::Method,
    headers: HeaderMap,
) -> Response {
    inner.request_count.fetch_add(1, Ordering::Relaxed);
    let count = {
        let mut m = inner.per_name_requests.lock();
        let c = m.entry(name.clone()).or_insert(0);
        *c += 1;
        *c
    };
    inner.log.lock().push(RequestRecord {
        method: method.to_string(),
        path: format!("/file/{name}"),
        range: headers
            .get(header::RANGE)
            .and_then(|v| v.to_str().ok())
            .map(String::from),
        user_agent: headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .map(String::from),
        referer: headers
            .get(header::REFERER)
            .and_then(|v| v.to_str().ok())
            .map(String::from),
        authorization: headers.contains_key(header::AUTHORIZATION),
    });

    let gd = inner.global_delay.load(Ordering::Relaxed);
    if gd > 0 {
        tokio::time::sleep(Duration::from_millis(gd as u64)).await;
    }
    if let Some(ms) = q.get("delay_ms").and_then(|v| v.parse::<u64>().ok()) {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }

    let size: u64 = q
        .get("size")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1024 * 1024);

    if let Some(n) = q.get("fail_first").and_then(|v| v.parse::<u64>().ok()) {
        if count <= n {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    }
    if let Some(code) = q.get("status").and_then(|v| v.parse::<u16>().ok()) {
        return StatusCode::from_u16(code)
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
            .into_response();
    }
    if let Some(n) = q.get("redirect").and_then(|v| v.parse::<u32>().ok()) {
        if n > 0 {
            let mut nq = q.clone();
            nq.insert("redirect".into(), (n - 1).to_string());
            let qs: Vec<String> = nq.iter().map(|(k, v)| format!("{k}={v}")).collect();
            let loc = format!("/file/{name}?{}", qs.join("&"));
            return (StatusCode::FOUND, [(header::LOCATION, loc)]).into_response();
        }
    }
    if let Some(cred) = q.get("auth") {
        let expected = format!(
            "Basic {}",
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, cred)
        );
        let ok = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .map(|v| v == expected)
            .unwrap_or(false);
        if !ok {
            return (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Basic realm=\"test\"")],
            )
                .into_response();
        }
    }
    if let Some(max) = q.get("max_conn").and_then(|v| v.parse::<usize>().ok()) {
        let cur = *inner.in_flight.lock().get(&name).unwrap_or(&0);
        if cur >= max {
            return (StatusCode::TOO_MANY_REQUESTS, [(header::RETRY_AFTER, "1")]).into_response();
        }
    }
    *inner.in_flight.lock().entry(name.clone()).or_insert(0) += 1;
    let guard = InFlightGuard(inner.clone(), name.clone());

    let etag = q
        .get("etag")
        .cloned()
        .unwrap_or_else(|| etag_for(&name, size));
    let norange = q.get("norange").map(|v| v == "1").unwrap_or(false);
    let nolength = q.get("nolength").map(|v| v == "1").unwrap_or(false);
    let ctype = q
        .get("ctype")
        .cloned()
        .unwrap_or_else(|| "application/octet-stream".into());
    let throttle: Option<u64> = q.get("throttle").and_then(|v| v.parse().ok());
    let fail_after: Option<u64> = q.get("fail_after").and_then(|v| v.parse().ok());
    let corrupt = q.get("corrupt").map(|v| v == "1").unwrap_or(false);

    let mut full = content_for(&name, size);
    if corrupt && size > 0 {
        let mut v = full.to_vec();
        let idx = (size / 2) as usize;
        v[idx] ^= 0xFF;
        full = Bytes::from(v);
    }

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(header::CONTENT_TYPE, HeaderValue::from_str(&ctype).unwrap());
    resp_headers.insert(header::ETAG, HeaderValue::from_str(&etag).unwrap());
    resp_headers.insert(
        header::LAST_MODIFIED,
        HeaderValue::from_static("Wed, 01 Jan 2025 00:00:00 GMT"),
    );
    if let Some(d) = q.get("disposition") {
        resp_headers.insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&format!("attachment; filename=\"{d}\"")).unwrap(),
        );
    }
    if !norange {
        resp_headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    }

    let (status, start, end) = match headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
        Some(r) if !norange => match parse_range(r, size) {
            Some((s, e)) => (StatusCode::PARTIAL_CONTENT, s, e + 1),
            None => {
                resp_headers.insert(
                    header::CONTENT_RANGE,
                    HeaderValue::from_str(&format!("bytes */{size}")).unwrap(),
                );
                return (StatusCode::RANGE_NOT_SATISFIABLE, resp_headers).into_response();
            }
        },
        _ => (StatusCode::OK, 0, size),
    };
    if status == StatusCode::PARTIAL_CONTENT {
        resp_headers.insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes {start}-{}/{size}", end - 1)).unwrap(),
        );
    }
    if !nolength {
        resp_headers.insert(header::CONTENT_LENGTH, HeaderValue::from(end - start));
    }
    if method == axum::http::Method::HEAD {
        drop(guard);
        return (status, resp_headers).into_response();
    }

    let body_bytes = full.slice(start as usize..end as usize);
    let stream = async_stream_body(body_bytes, throttle, fail_after, guard);
    let mut resp = Response::new(Body::from_stream(stream));
    *resp.status_mut() = status;
    *resp.headers_mut() = resp_headers;
    resp
}

fn async_stream_body(
    data: Bytes,
    throttle: Option<u64>,
    fail_after: Option<u64>,
    guard: InFlightGuard,
) -> impl futures::Stream<Item = Result<Bytes, std::io::Error>> {
    let chunk = match throttle {
        Some(t) if t < 64 * 1024 => 4 * 1024usize,
        _ => 64 * 1024usize,
    };
    futures::stream::unfold((data, 0u64, guard), move |(data, sent, guard)| async move {
        if sent as usize >= data.len() {
            return None;
        }
        if let Some(f) = fail_after {
            if sent >= f {
                return Some((
                    Err(std::io::Error::new(
                        std::io::ErrorKind::ConnectionReset,
                        "simulated disconnect",
                    )),
                    (data, sent, guard),
                ));
            }
        }
        let end = ((sent as usize) + chunk).min(data.len());
        let end = match fail_after {
            Some(f) if (f as usize) > sent as usize && (f as usize) < end => f as usize,
            _ => end,
        };
        let piece = data.slice(sent as usize..end);
        if let Some(t) = throttle {
            let secs = piece.len() as f64 / t.max(1) as f64;
            tokio::time::sleep(Duration::from_secs_f64(secs)).await;
        }
        Some((Ok(piece), (data, end as u64, guard)))
    })
}

async fn serve_html(State(inner): State<Arc<Inner>>, Path(name): Path<String>) -> Response {
    inner.request_count.fetch_add(1, Ordering::Relaxed);
    inner.log.lock().push(RequestRecord {
        method: "GET".into(),
        path: format!("/html/{name}"),
        range: None,
        user_agent: None,
        referer: None,
        authorization: false,
    });
    match inner.html.lock().get(&name) {
        Some(h) => (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            h.clone(),
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn serve_robots(State(inner): State<Arc<Inner>>) -> Response {
    inner.request_count.fetch_add(1, Ordering::Relaxed);
    match inner.text.lock().get("robots.txt") {
        Some((_, body)) => ([(header::CONTENT_TYPE, "text/plain")], body.clone()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn serve_text(State(inner): State<Arc<Inner>>, Path(name): Path<String>) -> Response {
    inner.request_count.fetch_add(1, Ordering::Relaxed);
    inner.log.lock().push(RequestRecord {
        method: "GET".into(),
        path: format!("/text/{name}"),
        range: None,
        user_agent: None,
        referer: None,
        authorization: false,
    });
    match inner.text.lock().get(&name) {
        Some((ct, body)) => ([(header::CONTENT_TYPE, ct.clone())], body.clone()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn serve_bytes(
    State(inner): State<Arc<Inner>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Response {
    inner.request_count.fetch_add(1, Ordering::Relaxed);
    inner.log.lock().push(RequestRecord {
        method: "GET".into(),
        path: format!("/bytes/{name}"),
        range: headers
            .get(header::RANGE)
            .and_then(|v| v.to_str().ok())
            .map(String::from),
        user_agent: None,
        referer: None,
        authorization: false,
    });
    let entry = inner.bytes.lock().get(&name).cloned();
    match entry {
        Some((body, ct)) => {
            let size = body.len() as u64;
            let mut h = HeaderMap::new();
            h.insert(header::CONTENT_TYPE, HeaderValue::from_str(&ct).unwrap());
            h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
            if let Some((s, e)) = headers
                .get(header::RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(|r| parse_range(r, size))
            {
                h.insert(
                    header::CONTENT_RANGE,
                    HeaderValue::from_str(&format!("bytes {s}-{e}/{size}")).unwrap(),
                );
                return (
                    StatusCode::PARTIAL_CONTENT,
                    h,
                    body.slice(s as usize..=e as usize),
                )
                    .into_response();
            }
            (StatusCode::OK, h, body).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn stats(State(inner): State<Arc<Inner>>) -> Response {
    let per: HashMap<String, u64> = inner.per_name_requests.lock().clone();
    axum::Json(serde_json::json!({ "requests": inner.request_count.load(Ordering::Relaxed), "per_name": per })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn serves_ranges_and_full() {
        let s = TestServer::start().await;
        let url = s.file_url("a.bin", 100_000);
        let client = reqwest_lite::get(&url, None).await;
        assert_eq!(client.0, 200);
        assert_eq!(client.1.len(), 100_000);
        let part = reqwest_lite::get(&url, Some("bytes=10-19")).await;
        assert_eq!(part.0, 206);
        assert_eq!(part.1, content_for("a.bin", 100_000).slice(10..20));
        assert!(s
            .requests()
            .iter()
            .any(|r| r.range.as_deref() == Some("bytes=10-19")));
    }

    /// Tiny HTTP/1.1 client so the test server's own tests do not depend on reqwest.
    mod reqwest_lite {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        pub async fn get(url: &str, range: Option<&str>) -> (u16, bytes::Bytes) {
            let u = url.strip_prefix("http://").unwrap();
            let (hostport, path) = u.split_once('/').unwrap();
            let mut s = tokio::net::TcpStream::connect(hostport).await.unwrap();
            let mut req =
                format!("GET /{path} HTTP/1.1\r\nHost: {hostport}\r\nConnection: close\r\n");
            if let Some(r) = range {
                req.push_str(&format!("Range: {r}\r\n"));
            }
            req.push_str("\r\n");
            s.write_all(req.as_bytes()).await.unwrap();
            let mut buf = Vec::new();
            s.read_to_end(&mut buf).await.unwrap();
            let sep = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
            let head = String::from_utf8_lossy(&buf[..sep]).to_string();
            let status: u16 = head.split_whitespace().nth(1).unwrap().parse().unwrap();
            (status, bytes::Bytes::copy_from_slice(&buf[sep + 4..]))
        }
    }
}
