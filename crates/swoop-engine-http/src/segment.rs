//! One worker = one connection = one attempt at one segment.
//!
//! A worker never retries by itself: on any failure it reports back and the controller decides
//! (backoff, mirror switch, shrink, give up). That keeps every policy decision in one place
//! and lets a failure change the concurrency, which a worker cannot see.

use crate::classify::{classify_reqwest, classify_status};
use crate::hostlimits::HostPermit;
use crate::plan::{SegStatus, UNKNOWN_END};
use crate::probe::parse_content_range;
use crate::shared::RunShared;
use futures::StreamExt;
use http::header::{
    CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, LAST_MODIFIED, RANGE, RETRY_AFTER,
};
use http::StatusCode;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use swoop_domain::events::LogLevel;
use swoop_domain::{ErrorKind, TaskError};
use swoop_runtime::engine::EngineStat;
use swoop_runtime::redact::redact;
use tokio_util::sync::CancellationToken;

/// Where a failure came from; decides whether a mirror switch can help.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureOrigin {
    /// Transport or protocol problem talking to the mirror.
    Network,
    /// The server answered with a status the request cannot proceed with.
    Server,
    /// Local disk — no mirror will fix it.
    Disk,
}

#[derive(Debug)]
pub enum WorkerResult {
    Done,
    /// Stopped by pause/cancel or by the controller (shrink); remainder stays pending.
    Stopped,
    Failed {
        error: TaskError,
        origin: FailureOrigin,
    },
    /// The server ignored `Range` on a request that needed it.
    RangeUnsupported,
}

#[derive(Debug)]
pub struct WorkerOutcome {
    pub seg: usize,
    pub mirror: usize,
    pub result: WorkerResult,
}

struct ActiveGuard(Arc<RunShared>);
impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0
            .control
            .counters
            .active_connections
            .fetch_sub(1, Ordering::Relaxed);
    }
}

/// Run one attempt. `permit` is held until the connection is closed.
pub async fn run_worker(
    shared: Arc<RunShared>,
    seg: usize,
    mirror: usize,
    token: CancellationToken,
    _permit: HostPermit,
) -> WorkerOutcome {
    let result = attempt(&shared, seg, mirror, &token).await;
    WorkerOutcome {
        seg,
        mirror,
        result,
    }
}

async fn attempt(
    shared: &Arc<RunShared>,
    seg: usize,
    mirror: usize,
    token: &CancellationToken,
) -> WorkerResult {
    let control = shared.control.clone();
    let (written, end, start, single) = {
        let plan = shared.plan.lock();
        let Some(s) = plan.segments.get(seg) else {
            return WorkerResult::Failed {
                error: TaskError::internal("segment handle out of range"),
                origin: FailureOrigin::Disk,
            };
        };
        (s.written, s.end, s.start, plan.segments.len() == 1)
    };
    if end != UNKNOWN_END && written >= end {
        return WorkerResult::Done;
    }
    let url = shared.mirrors.lock().url(mirror).to_owned();
    let ranges = shared.ranges();

    let mut rb = shared.template.apply(shared.client.get(&url));
    if ranges {
        let range = if end == UNKNOWN_END {
            format!("bytes={written}-")
        } else {
            format!("bytes={written}-{}", end - 1)
        };
        rb = rb.header(RANGE, range);
    }
    shared.log(
        LogLevel::Debug,
        "segment.start",
        format!("segment {seg} mirror {mirror} offset {written} end {end}"),
    );

    let resp = tokio::select! {
        _ = control.stopped() => return WorkerResult::Stopped,
        _ = token.cancelled() => return WorkerResult::Stopped,
        r = rb.send() => r,
    };
    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            shared.stat(EngineStat::ConnectionFailed);
            let error = classify_reqwest(&e, &url);
            if error.kind == ErrorKind::ConnectionReset {
                shared.reset_events.fetch_add(1, Ordering::Relaxed);
            }
            return WorkerResult::Failed {
                error,
                origin: FailureOrigin::Network,
            };
        }
    };

    // --- status / header validation -------------------------------------------------------
    let status = resp.status();
    let header = |name: &http::HeaderName| -> Option<String> {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim().to_owned())
    };
    let mut expect_len: Option<u64> = if end == UNKNOWN_END {
        None
    } else {
        Some(end - written)
    };
    match status {
        StatusCode::PARTIAL_CONTENT => {
            let Some((rs, re, rtotal)) =
                header(&CONTENT_RANGE).and_then(|v| parse_content_range(&v))
            else {
                return fail_source_changed("206 without a valid Content-Range", &url);
            };
            if rs != written {
                return fail_source_changed(
                    &format!("Content-Range starts at {rs}, expected {written}"),
                    &url,
                );
            }
            if let (Some(t), Some(expected)) = (rtotal, shared.expected_total) {
                if t != expected {
                    return fail_source_changed(
                        &format!("size changed from {expected} to {t}"),
                        &url,
                    );
                }
            }
            if end != UNKNOWN_END && re + 1 < end && single {
                // Server capped the range short of what we asked for; treat as truncated later.
                expect_len = Some(re + 1 - written);
            }
        }
        StatusCode::OK => {
            if ranges && !(written == 0 && start == 0 && single) {
                // A 200 to a mid-file range means the server ignored it.
                return WorkerResult::RangeUnsupported;
            }
            if ranges && written == 0 && single {
                // Whole-file 200 to a `bytes=0-` request: acceptable, but no resume here.
                shared.ranges.store(false, Ordering::Relaxed);
                shared.stat(EngineStat::RangeSupport { supported: false });
                shared.log(
                    LogLevel::Warn,
                    "http.range_unsupported",
                    "server answered 200 to a range request; continuing without resume",
                );
            }
            if let (Some(len), Some(expected)) = (
                header(&CONTENT_LENGTH).and_then(|v| v.parse::<u64>().ok()),
                shared.expected_total,
            ) {
                if len != expected {
                    return fail_source_changed(
                        &format!("size changed from {expected} to {len}"),
                        &url,
                    );
                }
            }
        }
        s => {
            let retry_after = header(&RETRY_AFTER);
            let error = classify_status(s.as_u16(), &url, retry_after.as_deref());
            if matches!(
                error.kind,
                ErrorKind::Throttled | ErrorKind::ServerError | ErrorKind::QuotaExceeded
            ) {
                shared.throttle_events.fetch_add(1, Ordering::Relaxed);
                shared.stat(EngineStat::Throttled);
            }
            shared.stat(EngineStat::ConnectionFailed);
            return WorkerResult::Failed {
                error,
                origin: FailureOrigin::Server,
            };
        }
    }
    if shared.validate_validators {
        if let (Some(mine), Some(theirs)) = (&shared.etag, header(&ETAG)) {
            if *mine != theirs {
                return fail_source_changed("ETag changed", &url);
            }
        }
        if let (Some(mine), Some(theirs)) = (&shared.last_modified, header(&LAST_MODIFIED)) {
            if *mine != theirs {
                return fail_source_changed("Last-Modified changed", &url);
            }
        }
    }
    if shared.expect_binary && written == 0 {
        let ct = header(&CONTENT_TYPE)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if ct.starts_with("text/html") || ct.starts_with("application/xhtml") {
            return WorkerResult::Failed {
                error: TaskError::new(
                    ErrorKind::UnexpectedContent,
                    "server sent an HTML page instead of the file",
                )
                .with_source(redact(&url))
                .with_detail(format!("content-type {ct}")),
                origin: FailureOrigin::Server,
            };
        }
    }

    shared.stat(EngineStat::ConnectionOpened);
    control
        .counters
        .active_connections
        .fetch_add(1, Ordering::Relaxed);
    let _guard = ActiveGuard(shared.clone());

    // --- streaming ----------------------------------------------------------------------
    let mut stream = resp.bytes_stream();
    let mut received: u64 = 0;
    loop {
        let item = tokio::select! {
            _ = control.stopped() => return WorkerResult::Stopped,
            _ = token.cancelled() => return WorkerResult::Stopped,
            it = stream.next() => it,
        };
        let chunk = match item {
            None => break,
            Some(Ok(c)) => c,
            Some(Err(e)) => {
                let error = classify_reqwest(&e, &url);
                if matches!(
                    error.kind,
                    ErrorKind::ConnectionReset | ErrorKind::Truncated
                ) {
                    shared.reset_events.fetch_add(1, Ordering::Relaxed);
                }
                return WorkerResult::Failed {
                    error,
                    origin: FailureOrigin::Network,
                };
            }
        };
        if chunk.is_empty() {
            continue;
        }
        received += chunk.len() as u64;
        // The segment may have been split under us: keep only what is still ours.
        let (offset, cur_end) = {
            let plan = shared.plan.lock();
            match plan.segments.get(seg) {
                Some(s) if s.status == SegStatus::Running => (s.written, s.end),
                _ => return WorkerResult::Stopped,
            }
        };
        if cur_end != UNKNOWN_END && offset >= cur_end {
            break;
        }
        let keep = if cur_end == UNKNOWN_END {
            chunk.len()
        } else {
            (chunk.len() as u64).min(cur_end - offset) as usize
        };
        let data = if keep < chunk.len() {
            shared.stat(EngineStat::BytesDiscarded {
                bytes: (chunk.len() - keep) as u64,
            });
            chunk.slice(..keep)
        } else {
            chunk
        };
        let len = data.len() as u64;
        tokio::select! {
            _ = control.stopped() => return WorkerResult::Stopped,
            _ = token.cancelled() => return WorkerResult::Stopped,
            _ = shared.limiter.acquire(len) => {},
        }
        let write = tokio::select! {
            _ = control.stopped() => return WorkerResult::Stopped,
            _ = token.cancelled() => return WorkerResult::Stopped,
            w = shared.writer.write(offset, data) => w,
        };
        if let Err(e) = write {
            return WorkerResult::Failed {
                error: disk_error(e, &url),
                origin: FailureOrigin::Disk,
            };
        }
        let accepted = shared.plan.lock().advance(seg, offset, len);
        control.counters.add_downloaded(accepted);
        if cur_end != UNKNOWN_END && offset + accepted >= cur_end {
            break;
        }
    }

    let (written_now, end_now) = {
        let plan = shared.plan.lock();
        plan.segments
            .get(seg)
            .map(|s| (s.written, s.end))
            .unwrap_or((0, 0))
    };
    if end_now != UNKNOWN_END && written_now < end_now {
        // EOF before the range was complete.
        shared.reset_events.fetch_add(1, Ordering::Relaxed);
        return WorkerResult::Failed {
            error: TaskError::new(
                ErrorKind::Truncated,
                format!(
                    "connection closed after {received} bytes, {} missing",
                    end_now - written_now
                ),
            )
            .with_source(redact(&url))
            .with_detail(format!("expected {:?} bytes", expect_len)),
            origin: FailureOrigin::Network,
        };
    }
    WorkerResult::Done
}

fn fail_source_changed(detail: &str, url: &str) -> WorkerResult {
    WorkerResult::Failed {
        error: TaskError::new(ErrorKind::SourceChanged, "the file changed on the server")
            .with_detail(detail.to_owned())
            .with_source(redact(url)),
        origin: FailureOrigin::Server,
    }
}

/// The writer already classified its `io::Error` with `IoContext::Disk`; keep the kind and
/// attach the source URL so every task error carries one.
fn disk_error(e: TaskError, url: &str) -> TaskError {
    e.with_source(redact(url))
}
