# osprey-engine-http

The HTTP/HTTPS transfer engine: segmented downloads with adaptive concurrency, work-stealing,
mirrors and Metalink. It implements `osprey_runtime::engine::Transfer` for `TaskKind::Http`
and `TaskKind::Metalink` and never touches persistence or the UI.

## Public API

```rust
use osprey_engine_http::{HttpEngine, HostLimits, Tuning, MetalinkFile};

let engine = HttpEngine::new();                       // production defaults
let engine = HttpEngine::new()
    .with_tuning(Tuning { .. })                       // timings / thresholds (tests)
    .with_limits(Arc::new(HostLimits::new(16, 64)));  // private connection caps (tests)

// Transfer trait: engine.run(ctx).await / engine.probe(&task, settings, secrets, clients).await
// Bare-URL probe for the add dialog and media detection:
let meta = HttpEngine::probe_url(url, settings, clients, &secrets).await?;
// Metalink v3/v4 parsing (first <file> only):
let file: MetalinkFile = osprey_engine_http::metalink::parse(document)?;
```

`HostLimits::global()` is the process-wide registry of per-host / total connection semaphores
that every `HttpEngine::new()` shares; it is re-configured from the settings snapshot at the
start of each run.

## How a run works

1. **Resolve sources.** `Source::Urls` (one URL, or several mirrors of identical content) or
   `Source::Metalink` (inline document or fetched, ≤ 4 MiB, parsed with `quick-xml`; only
   `http`/`https` URLs become mirrors). Any other scheme is refused before a socket is opened
   (`UnsupportedScheme`).
2. **Probe.** `HEAD`, falling back to `GET Range: bytes=0-0` when HEAD is refused (405/403/
   501/…) or when it leaves a question open — no `Accept-Ranges`, no size, or a
   `Content-Length: 0` (servers answer HEAD with an empty body for dynamic content; a zero is
   only believed after the ranged GET confirms it). Mirrors are probed in parallel with a 5 s
   timeout; those whose size differs from the first responder are dropped (ETags legitimately
   differ across hosts). Transient probe failures are retried with the backoff policy.
   The probe succeeds for HTML too — the add dialog wants a MIME type; `UnexpectedContent` is
   raised only by `run` when a binary was expected (URL not `.html`/`.htm`, no
   `Content-Disposition`) and the body is `text/html`.
3. **Plan.**
   * Fresh: `N = control.max_connections()` (0 → `connections_per_task`), capped by
     `max_connections_per_host`, 64, and `total / min_segment_size`; files under
     `2 × min_segment_size` get one connection. The file is split into `N` equal ranges.
   * Resume: the `Checkpoint::Segments` map is validated against a fresh probe — size must
     match; ETag / Last-Modified must match when both sides have them (single source only) —
     otherwise `SourceChanged` (class `RestartFromScratch`, the services layer restarts). Every
     segment continues from its `committed` watermark. A missing part file starts over.
   * No range support (probe says so, or a mid-file range request gets a `200`): one
     connection from byte 0, no resume, `EngineStat::RangeSupport { supported: false }`.
     The `200`-to-a-range case is detected by a worker and the run is restarted in single
     mode with the part file truncated.
4. **Transfer.** The controller owns a `JoinSet` of workers; each worker is one connection
   making one attempt at one segment (`GET Range: bytes=written-end-1`, verified `206` +
   `Content-Range` start/total, ETag/Last-Modified re-checked on every response for a single
   source). Chunks pass through `limiter.acquire(len)` and `FileWriter::write(offset, chunk)`
   untouched (`Bytes` straight through), bump `control.counters`, and advance the segment's
   `written` offset. Every await is selected against `control.stopped()`.
5. **Durability.** Every `checkpoint_interval` (3 s) — and on pause/cancel — the controller
   snapshots each segment's `written`, performs **one** `flush(Barrier)` for the file, advances
   `committed` to the snapshot and emits `Checkpoint::Segments`. `committed` never moves any
   other way; a failed flush keeps the previous checkpoint (invariants 1–2 of
   `docs/architecture/002-persistence-and-recovery.md`).
6. **Completion.** `flush(Full)`, close, verify the strongest Metalink hash if there is one
   (`ChecksumMismatch` on failure; `task.options.checksum` belongs to the services layer),
   quarantine attribute when enabled, then an atomic rename `.osprey-part → name`
   (`unique_path` if the target appeared meanwhile; both paths are checked with
   `safety::ensure_within(task.directory, …)`).

## Adaptive concurrency (the controller)

The controller samples the aggregate rate every `adapt_interval` (2 s) and keeps a `target`
connection count. Whenever `running < target` a slot is filled with the largest ready pending
segment, or — when nothing is pending — by **work-stealing**: the largest running segment is
split at the midpoint of its remaining range (only when both halves stay ≥ `min_segment_size`)
and a new connection takes the second half (`EngineStat::SegmentReassigned`, log
`segment.split`). The worker that owns the first half notices its new `end` on the next chunk
and stops there.

Decisions, in the order they are evaluated on each sample:

| signal | decision |
|---|---|
| `429` / `503` / `509` / `Retry-After` on any connection | immediate: `target = running − 1` (min 1), the segment is re-queued not before `Retry-After` (or the throttled backoff), growth blocked for `throttle_cooldown` (30 s). Log `adaptive.shrink`, stat `Throttled`. |
| ≥ 2 connection resets / truncations within one sample | `target −= 1`, growth cooldown. |
| user set an explicit connection count (`control.max_connections() > 0`) | `target` follows it (a throttle may lower it temporarily); no growth experiments. |
| `running ≥ 3` and the aggregate is ≤ 1.15 × what `≤ running/2` connections achieved earlier | **throttling detected** (per-client cap: more connections only slice the same pipe thinner): shrink to that count, log `throttle.detected`, stat `Throttled`, cooldown 30 s. |
| a growth experiment is pending and `settle_samples` (2) have passed | keep the extra connection if the aggregate gained ≥ `grow_gain_threshold` (15 %), otherwise revert (`target −= 1`) and block growth for `grow_cooldown` (20 s). |
| adaptive enabled, no experiment pending, not cooling down, all slots busy, `target < max_connections_per_host`, something splittable remains | start an experiment: `target += 1` (log `adaptive.grow`). |
| nothing pending, ≥ 2 running, one segment < median speed / `slow_segment_factor` (4) | endgame: split the straggler so a faster connection can take its tail. |
| aggregate fell below 50 % of the previous sample | stat `ThroughputDrop` (diagnostics only). |

Per-connection failures that are not throttles (DNS, TLS, reset, timeout, truncated, 5xx) go
through `BackoffPolicy::delay_for_with_hint` with the error's `FailureClass` and
`retry_after_ms`; the segment is re-queued not before that delay (`EngineStat::Retry`, log
`segment.retry`). The retry budget resets whenever an attempt made progress, so a long
download survives many spaced-out disconnects. `Permanent`, `NeedsUser`, `RestartFromScratch`
and `WaitForCondition` classes, exhausted budgets, and any disk error fail the task.

### Mirrors

Segments are handed to mirrors by smooth weighted round-robin (`picks / weight`); the weight
starts as a function of probe latency and becomes the measured per-mirror throughput once
bytes flow. A failure on a mirror counts against it (3 strikes, or one non-retryable error,
disable it for the run) and moves the segment to the next healthy mirror
(`EngineStat::MirrorSwitched`, log `mirror.switch`). When none is left the task fails with
`MirrorExhausted`. Validators (ETag/Last-Modified) are not compared across mirrors, sizes are.

### Connection caps

`HostLimits` holds one semaphore per host (`max_connections_per_host`) and one global
(`max_total_connections`), shared by all tasks in the process. A worker holds one permit of
each for the life of its connection. Growth never blocks; only a task's very first connection
waits for a permit (selected against stop). The absolute ceiling is 64 connections.

## Pause / cancel

`control.pause` or `control.cancel` → every worker stops at its next await (they all select
on `control.stopped()`), the writer is flushed with `Barrier` under a 5 s timeout, a
checkpoint is emitted **only if that flush succeeded**, and `run` returns
`control.stop_outcome()`. Cancel keeps the part file; the services layer decides its fate.
`control.volume_lost` is treated as a pause.

## Progress and logs

`sink.progress` is called from the controller at ≤ 4 Hz (`progress_interval`, plus one final
line at completion) with downloaded/total/speed (`SpeedMeter`, ticked once per second)/
instant speed/ETA/active connections. Per-chunk work only bumps `control.counters`.
States: `Connecting` → `Downloading` (→ `Retrying` while a probe backs off) → `Verifying`
(metalink hash). Stable log codes: `http.probe`, `http.resume`, `http.range_unsupported`,
`http.verify`, `http.complete`, `http.pause`, `segment.start`, `segment.split`,
`segment.retry`, `mirror.switch`, `adaptive.grow`, `adaptive.shrink`, `throttle.detected`,
`metalink.multi_file`. Every message passes through `osprey_runtime::redact::redact`.

## Requests

`task.options.headers` are validated by `osprey_runtime::net::validate_headers` (no hop-by-hop
or engine-managed names, no CR/LF) — so are credential headers from `secrets.headers`.
`Accept-Encoding: identity` is always sent (content-coding breaks `Range` and
`Content-Length`). `user_agent`, `referer`, `cookies` (as a `Cookie` header), Basic auth from
`secrets.username/password`. The client comes from `ClientFactory::client(&ClientProfile)`:
HTTP/1.1-only unless `allow_http2` says otherwise (each segment must own a TCP connection),
proxy from `secrets.proxy_url`, `direct_connection`, TLS exception only for hosts on
`settings.network.tls_exceptions`. After a redirect, data requests use the final URL only when
it stays on the same host; a cross-host redirect keeps the original URL so origin-bound
credentials are never sent to a CDN directly.

## Tuning knobs

| knob | default | where |
|---|---|---|
| connections per task, per host, total, min segment size, adaptive on/off | 8 / 16 / 64 / 1 MiB / on | `settings.network` |
| retry base/max delay, max retries | 1 s / 60 s / 8 | `settings.network`, `task.options.max_retries` |
| connect / read timeout | 20 s / 45 s | `settings.network` (applied by `ClientFactory`) |
| preallocate, sparse, temp suffix, quarantine | on / off / `.osprey-part` / on | `settings.storage`, `task.options` |
| `progress_interval`, `checkpoint_interval`, `adapt_interval` | 250 ms / 3 s / 2 s | `Tuning` |
| `probe_timeout` (mirrors), `stop_flush_timeout` | 5 s / 5 s | `Tuning` |
| `grow_gain_threshold`, `settle_samples`, `grow_cooldown`, `throttle_cooldown`, `slow_segment_factor` | 15 % / 2 / 20 s / 30 s / 4 | `Tuning` |

## File naming

A locked task name wins; then the Metalink `<file name>`; then an explicit
`Content-Disposition`; then the task's own name; then the probe's URL/MIME-derived guess.
Everything is sanitised by `osprey_runtime::safety::sanitize_filename`, and on resume the
checkpoint's `part_path` is authoritative so the part file never moves between runs.

## Limitations

* **Multi-file Metalinks** download only the first `<file>` (a warning is logged with code
  `metalink.multi_file`; `MetalinkFile::file_count` tells the caller). Expanding a metalink
  into one task per file belongs to the services layer. Metalink `<pieces>` hashes and
  `<metaurl>` (torrent) entries are ignored.
* **Unknown sizes** (chunked responses without a usable `Content-Range`) are downloaded on a
  single connection without resume; a retry restarts from byte 0.
* **Throttling detection** compares aggregate rates across connection counts observed in the
  same run; it needs at least three connections and a few samples, so a very short download
  is never diagnosed (nor does it need to be).
* **Mirror re-probing:** a mirror disabled during a run is not retried until the next run.
* A failed mirror's per-connection permit accounting is per host of the *primary* URL; mirrors
  on other hosts count against the primary's cap rather than their own.
* HTTP/2 multiplexing is off by default on purpose; enabling it makes all segments share one
  TCP connection and removes most of the benefit of segmentation on high-latency links.
* Per-task `checksum` options are verified by the services layer, not here.
