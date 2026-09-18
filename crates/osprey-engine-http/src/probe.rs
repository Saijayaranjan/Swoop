//! Resolving a URL without downloading it: HEAD first, then a one-byte ranged GET when HEAD
//! is refused or leaves the important questions (size? ranges?) unanswered.
//!
//! The probe is deliberately tolerant: an HTML page probes fine (the add dialog wants to show
//! *something*); refusing HTML when a binary was expected is a decision for [`crate::engine`].

use crate::classify::{classify_reqwest, classify_status};
use crate::request::RequestTemplate;
use http::header::{
    ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG,
    LAST_MODIFIED, RANGE, RETRY_AFTER, SERVER,
};
use http::StatusCode;
use osprey_domain::TaskError;
use osprey_runtime::engine::ResolvedMetadata;
use osprey_runtime::filename;
use osprey_runtime::redact::redact;
use std::time::{Duration, Instant};

/// What a probe learned, in engine terms (the public `ResolvedMetadata` plus internals).
#[derive(Clone, Debug, Default)]
pub struct ProbeResult {
    pub metadata: ResolvedMetadata,
    /// The URL the data requests should use (after redirects).
    pub final_url: String,
    pub content_type: Option<String>,
    /// `Content-Disposition` was present (an explicit server-provided name).
    pub has_disposition: bool,
    /// Round-trip time of the successful probe request (mirror ranking).
    pub latency: Duration,
    /// Whether the server answered a `Range` request with 206.
    pub ranges_confirmed: bool,
}

impl ProbeResult {
    pub fn total(&self) -> Option<u64> {
        self.metadata.total
    }
    pub fn resumable(&self) -> bool {
        self.metadata.resumable.unwrap_or(false)
    }
    pub fn is_html(&self) -> bool {
        self.content_type
            .as_deref()
            .map(|c| c.starts_with("text/html") || c.starts_with("application/xhtml"))
            .unwrap_or(false)
    }
}

/// Probe `url`. `original_url` is what the user supplied (before redirects) and is preferred
/// as the source of the file name; the final URL after a CDN redirect usually carries a hash.
pub async fn probe(
    client: &reqwest::Client,
    template: &RequestTemplate,
    url: &str,
    timeout: Duration,
) -> Result<ProbeResult, TaskError> {
    let started = Instant::now();
    let head = template
        .apply(client.head(url))
        .timeout(timeout)
        .send()
        .await;
    match head {
        Ok(resp) if resp.status().is_success() => {
            let mut out = from_response(&resp, url, started.elapsed());
            drop(resp);
            // No Accept-Ranges header does not mean "no ranges", and `Content-Length: 0` on
            // HEAD usually means "no HEAD body" rather than an empty file (dynamic content,
            // chunked responses): confirm both with a real ranged GET.
            let unsure_size = matches!(out.metadata.total, None | Some(0));
            if out.metadata.resumable.is_none() || unsure_size {
                if let Ok(ranged) = range_probe(client, template, url, timeout).await {
                    merge_range_probe(&mut out, &ranged, unsure_size);
                }
            }
            if out.metadata.resumable.is_none() {
                out.metadata.resumable = Some(false);
            }
            Ok(out)
        }
        Ok(resp) => {
            let status = resp.status();
            // Servers that refuse HEAD (405/403/501/400) usually serve GET fine; anything else
            // (401/404/5xx) is the real answer, but confirm with GET before failing on HEAD
            // alone because some CDNs only misbehave on HEAD.
            let retry_after = resp
                .headers()
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .map(String::from);
            let head_error = classify_status(status.as_u16(), url, retry_after.as_deref());
            drop(resp);
            match range_probe(client, template, url, timeout).await {
                Ok(out) => Ok(out),
                Err(get_error) => {
                    // Prefer the GET verdict (it is what the download will see) unless it
                    // was a transport error masking a clear HTTP status from HEAD.
                    if get_error.status_code.is_some() || head_error.status_code.is_none() {
                        Err(get_error)
                    } else {
                        Err(head_error)
                    }
                }
            }
        }
        Err(e) => {
            // Transport failure on HEAD: try GET once — some proxies drop HEAD entirely.
            let head_err = classify_reqwest(&e, url);
            match range_probe(client, template, url, timeout).await {
                Ok(out) => Ok(out),
                Err(_) => Err(head_err),
            }
        }
    }
}

/// `GET` with `Range: bytes=0-0`; the body (one byte, or the whole file on a server that
/// ignores ranges) is dropped without being read.
async fn range_probe(
    client: &reqwest::Client,
    template: &RequestTemplate,
    url: &str,
    timeout: Duration,
) -> Result<ProbeResult, TaskError> {
    let started = Instant::now();
    let resp = template
        .apply(client.get(url))
        .header(RANGE, "bytes=0-0")
        .timeout(timeout)
        .send()
        .await
        .map_err(|e| classify_reqwest(&e, url))?;
    let status = resp.status();
    if status == StatusCode::PARTIAL_CONTENT {
        let mut out = from_response(&resp, url, started.elapsed());
        let total = resp
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_content_range)
            .and_then(|(_, _, total)| total);
        if total.is_some() {
            out.metadata.total = total;
        }
        out.metadata.resumable = Some(true);
        out.ranges_confirmed = true;
        return Ok(out);
    }
    if status.is_success() {
        let mut out = from_response(&resp, url, started.elapsed());
        out.metadata.resumable = Some(false);
        return Ok(out);
    }
    if status == StatusCode::RANGE_NOT_SATISFIABLE {
        // Zero-length resource: `bytes=0-0` cannot be satisfied. Ranges are still supported.
        let mut out = from_response(&resp, url, started.elapsed());
        let total = resp
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().strip_prefix("bytes */").map(str::to_owned))
            .and_then(|t| t.trim().parse::<u64>().ok());
        out.metadata.total = total.or(Some(0));
        out.metadata.resumable = Some(true);
        return Ok(out);
    }
    let retry_after = resp
        .headers()
        .get(RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    Err(classify_status(
        status.as_u16(),
        url,
        retry_after.as_deref(),
    ))
}

fn merge_range_probe(out: &mut ProbeResult, ranged: &ProbeResult, replace_size: bool) {
    if replace_size {
        // The GET saw the real body: its Content-Range total or Content-Length (or nothing,
        // for a chunked response) beats whatever HEAD claimed.
        out.metadata.total = ranged.metadata.total;
    }
    if out.metadata.resumable.is_none() {
        out.metadata.resumable = ranged.metadata.resumable;
    }
    out.ranges_confirmed = ranged.ranges_confirmed;
    if out.metadata.etag.is_none() {
        out.metadata.etag = ranged.metadata.etag.clone();
    }
    if out.metadata.last_modified.is_none() {
        out.metadata.last_modified = ranged.metadata.last_modified.clone();
    }
}

/// Build the result from response headers (shared by HEAD and GET paths).
fn from_response(resp: &reqwest::Response, original_url: &str, latency: Duration) -> ProbeResult {
    let h = resp.headers();
    let text = |name: &http::HeaderName| -> Option<String> {
        h.get(name)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
    };
    let final_url = resp.url().to_string();
    let content_type = text(&CONTENT_TYPE);
    let disposition = text(&CONTENT_DISPOSITION);
    let content_length = text(&CONTENT_LENGTH).and_then(|v| v.parse::<u64>().ok());
    // Prefer the total from Content-Range when present (206 answers), else Content-Length —
    // but only for full responses: a partial response's Content-Length is the range size.
    let content_range_total = text(&CONTENT_RANGE)
        .and_then(|v| parse_content_range(&v))
        .and_then(|(_, _, t)| t);
    let total = if resp.status() == StatusCode::PARTIAL_CONTENT {
        content_range_total
    } else {
        content_length
    };
    let accept_ranges = text(&ACCEPT_RANGES).map(|v| v.to_ascii_lowercase());
    let resumable = match accept_ranges.as_deref() {
        Some(v) if v.contains("bytes") => Some(true),
        Some(_) => Some(false),
        None => None,
    };
    // The user's URL usually carries the name they clicked on; a CDN's final URL rarely does.
    let name_url = if filename::from_url(original_url).is_some() {
        original_url
    } else {
        final_url.as_str()
    };
    let name = filename::resolve(name_url, disposition.as_deref(), content_type.as_deref());
    let mime = content_type
        .as_deref()
        .map(|c| {
            c.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        })
        .filter(|m| !m.is_empty());
    let metadata = ResolvedMetadata {
        name: Some(name),
        total,
        mime,
        resumable,
        final_url: Some(redact(&final_url)),
        etag: text(&ETAG),
        last_modified: text(&LAST_MODIFIED),
        server: text(&SERVER),
        content_disposition: disposition.clone(),
        http_version: Some(format!("{:?}", resp.version())),
        remote_addr: resp.remote_addr().map(|a| a.to_string()),
        torrent: None,
        media: None,
        file_path: None,
    };
    ProbeResult {
        metadata,
        final_url,
        content_type,
        has_disposition: disposition.is_some(),
        latency,
        ranges_confirmed: false,
    }
}

/// Use the post-redirect URL for data requests when it stays on the same host (saves a
/// redirect round-trip per segment). A cross-host redirect keeps the original URL so
/// credentials/cookies bound to the origin are never sent to the CDN directly.
pub fn data_url(original: &str, final_url: &str) -> String {
    let same_host = match (url::Url::parse(original), url::Url::parse(final_url)) {
        (Ok(a), Ok(b)) => {
            a.host_str().map(|h| h.to_ascii_lowercase())
                == b.host_str().map(|h| h.to_ascii_lowercase())
        }
        _ => false,
    };
    if same_host {
        final_url.to_owned()
    } else {
        original.to_owned()
    }
}

/// Parse `bytes start-end/total` (`total` may be `*`). Returns `(start, end_inclusive, total)`.
pub fn parse_content_range(value: &str) -> Option<(u64, u64, Option<u64>)> {
    let v = value.trim();
    let rest = v.strip_prefix("bytes")?.trim_start();
    let (range, total) = rest.split_once('/')?;
    let total = match total.trim() {
        "*" => None,
        t => Some(t.parse::<u64>().ok()?),
    };
    let (s, e) = range.trim().split_once('-')?;
    let start = s.trim().parse::<u64>().ok()?;
    let end = e.trim().parse::<u64>().ok()?;
    if end < start {
        return None;
    }
    Some((start, end, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_url_keeps_origin_across_hosts() {
        assert_eq!(
            data_url("https://a/x", "https://a/y?sig=1"),
            "https://a/y?sig=1"
        );
        assert_eq!(data_url("https://a/x", "https://cdn/y"), "https://a/x");
    }

    #[test]
    fn content_range_parsing() {
        assert_eq!(
            parse_content_range("bytes 0-99/1000"),
            Some((0, 99, Some(1000)))
        );
        assert_eq!(parse_content_range("bytes 5-9/*"), Some((5, 9, None)));
        assert_eq!(parse_content_range("bytes 9-5/10"), None);
        assert_eq!(parse_content_range("items 0-1/2"), None);
        assert_eq!(parse_content_range("bytes */10"), None);
    }
}
