//! End-to-end tests of `HttpEngine` against the in-process test server. No public internet.

use osprey_domain::checkpoint::Checkpoint;
use osprey_domain::events::LogLevel;
use osprey_domain::settings::Settings;
use osprey_domain::{
    ErrorKind, FailureClass, Millis, Progress, QueueId, Source, Task, TaskKind, TaskState,
};
use osprey_engine_http::{HostLimits, HttpEngine, Tuning};
use osprey_runtime::engine::{
    EngineStat, ProgressSink, ResolvedMetadata, Transfer, TransferContext, TransferControl,
    TransferOutcome, TransferSecrets,
};
use osprey_runtime::net::ClientFactory;
use osprey_runtime::RateLimiter;
use osprey_testserver::{content_for, sha256_hex, TestServer};
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------------------------
// harness
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct Recorder {
    checkpoints: Mutex<Vec<Checkpoint>>,
    stats: Mutex<Vec<EngineStat>>,
    logs: Mutex<Vec<(LogLevel, String, String)>>,
    states: Mutex<Vec<TaskState>>,
    metadata: Mutex<Option<ResolvedMetadata>>,
    progress: Mutex<Vec<Progress>>,
}

impl Recorder {
    fn last_checkpoint(&self) -> Option<Checkpoint> {
        self.checkpoints.lock().last().cloned()
    }
    fn has_stat(&self, f: impl Fn(&EngineStat) -> bool) -> bool {
        self.stats.lock().iter().any(f)
    }
    fn has_log(&self, code: &str) -> bool {
        self.logs.lock().iter().any(|(_, c, _)| c == code)
    }
}

impl ProgressSink for Recorder {
    fn progress(&self, p: Progress) {
        self.progress.lock().push(p);
    }
    fn state(&self, s: TaskState, _: Option<String>) {
        self.states.lock().push(s);
    }
    fn metadata(&self, m: ResolvedMetadata) {
        *self.metadata.lock() = Some(m);
    }
    fn checkpoint(&self, c: Checkpoint) {
        self.checkpoints.lock().push(c);
    }
    fn log(&self, level: LogLevel, code: &str, message: String) {
        self.logs.lock().push((level, code.to_owned(), message));
    }
    fn stat(&self, s: EngineStat) {
        self.stats.lock().push(s);
    }
}

fn test_settings() -> Settings {
    let mut s = Settings::default();
    s.network.retry_base_delay_ms = 50;
    s.network.retry_max_delay_ms = 400;
    s.network.max_retries = 20;
    s.network.connections_per_task = 8;
    s.storage.quarantine_downloads = false;
    s
}

fn test_tuning() -> Tuning {
    Tuning {
        progress_interval: Duration::from_millis(50),
        checkpoint_interval: Duration::from_millis(200),
        adapt_interval: Duration::from_millis(200),
        probe_timeout: Duration::from_secs(5),
        stop_flush_timeout: Duration::from_secs(5),
        settle_samples: 2,
        grow_cooldown: Duration::from_millis(500),
        throttle_cooldown: Duration::from_millis(500),
        ..Tuning::default()
    }
}

fn engine() -> HttpEngine {
    HttpEngine::new()
        .with_tuning(test_tuning())
        .with_limits(Arc::new(HostLimits::new(16, 64)))
}

fn http_task(urls: Vec<String>, name: &str, dir: &Path) -> Task {
    Task::new(
        TaskKind::Http,
        Source::Urls { urls },
        name,
        dir.to_path_buf(),
        QueueId::default_queue(),
    )
}

struct Run {
    control: Arc<TransferControl>,
    sink: Arc<Recorder>,
    limiter: Arc<RateLimiter>,
    settings: Arc<Settings>,
    secrets: TransferSecrets,
    checkpoint: Option<Checkpoint>,
}

impl Run {
    fn new(settings: Settings) -> Self {
        Self {
            control: TransferControl::new(),
            sink: Arc::new(Recorder::default()),
            limiter: RateLimiter::unlimited("test"),
            settings: Arc::new(settings),
            secrets: TransferSecrets::default(),
            checkpoint: None,
        }
    }

    fn ctx(&self, task: Task) -> TransferContext {
        TransferContext {
            task,
            checkpoint: self.checkpoint.clone(),
            control: self.control.clone(),
            sink: self.sink.clone(),
            settings: self.settings.clone(),
            secrets: self.secrets.clone(),
            limiter: self.limiter.clone(),
            upload_limiter: RateLimiter::unlimited("up"),
            clients: ClientFactory::new(self.settings.clone()),
            started_at: Millis::now(),
        }
    }
}

async fn download(task: Task, run: &Run) -> TransferOutcome {
    engine().run(run.ctx(task)).await
}

fn assert_completed(outcome: &TransferOutcome) -> (PathBuf, u64) {
    match outcome {
        TransferOutcome::Completed { file_path, bytes } => (file_path.clone(), *bytes),
        other => panic!("expected Completed, got {other:?}"),
    }
}

/// Print the engine's log trail (shown by `cargo test` only on failure).
fn dump(run: &Run) {
    for (level, code, msg) in run.sink.logs.lock().iter() {
        eprintln!("[{level:?}] {code}: {msg}");
    }
    eprintln!("stats: {:?}", run.sink.stats.lock());
}

fn assert_failed(outcome: &TransferOutcome, kind: ErrorKind) -> osprey_domain::TaskError {
    match outcome {
        TransferOutcome::Failed(e) => {
            assert_eq!(e.kind, kind, "{e:?}");
            assert!(e.source_url.is_some(), "errors carry source_url: {e:?}");
            e.clone()
        }
        other => panic!("expected Failed({kind:?}), got {other:?}"),
    }
}

fn assert_file_matches(path: &Path, name: &str, size: u64) {
    let data = std::fs::read(path).expect("read final file");
    assert_eq!(data.len() as u64, size, "size");
    assert_eq!(
        sha256_hex(&data),
        sha256_hex(&content_for(name, size)),
        "content mismatch"
    );
}

fn range_start(r: &str) -> Option<u64> {
    r.strip_prefix("bytes=")?
        .split('-')
        .next()?
        .parse::<u64>()
        .ok()
}

// ---------------------------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_download_matches_checksum() {
    let server = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 3 * 1024 * 1024;
    let run = Run::new(test_settings());
    let task = http_task(
        vec![server.file_url("full.bin", size)],
        "full.bin",
        dir.path(),
    );
    let outcome = download(task, &run).await;
    let (path, bytes) = assert_completed(&outcome);
    assert_eq!(bytes, size);
    assert_eq!(path, dir.path().join("full.bin"));
    assert_file_matches(&path, "full.bin", size);
    assert!(!dir.path().join("full.bin.osprey-part").exists());
    let meta = run.sink.metadata.lock().clone().unwrap();
    assert_eq!(meta.total, Some(size));
    assert_eq!(meta.resumable, Some(true));
    assert_eq!(meta.name.as_deref(), Some("full.bin"));
    assert!(meta.http_version.is_some() && meta.remote_addr.is_some());
    assert!(run.sink.has_log("http.probe"));
    assert!(run.sink.has_log("http.complete"));
    assert!(run.sink.states.lock().contains(&TaskState::Downloading));
    assert!(run
        .sink
        .has_stat(|s| matches!(s, EngineStat::RangeSupport { supported: true })));
    let last = run.sink.progress.lock().last().cloned().unwrap();
    assert_eq!(last.downloaded, size);
    assert_eq!(last.total, Some(size));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn segmented_download_uses_multiple_ranges() {
    let server = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 8 * 1024 * 1024;
    let run = Run::new(test_settings());
    let task = http_task(
        vec![server.file_url("seg.bin", size)],
        "seg.bin",
        dir.path(),
    );
    let (path, _) = assert_completed(&download(task, &run).await);
    assert_file_matches(&path, "seg.bin", size);
    let ranged = server
        .requests()
        .iter()
        .filter(|r| r.method == "GET" && r.range.is_some())
        .count();
    assert!(ranged >= 4, "expected >= 4 range requests, saw {ranged}");
    assert!(run.sink.has_log("segment.start"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn norange_server_falls_back_to_single_connection() {
    let server = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 3 * 1024 * 1024;
    let run = Run::new(test_settings());
    let url = server.url(&format!("/file/nr.bin?size={size}&norange=1"));
    let task = http_task(vec![url], "nr.bin", dir.path());
    let (path, _) = assert_completed(&download(task, &run).await);
    assert_file_matches(&path, "nr.bin", size);
    assert!(run
        .sink
        .has_stat(|s| matches!(s, EngineStat::RangeSupport { supported: false })));
    assert!(run.sink.has_log("http.range_unsupported"));
    let gets = server
        .requests()
        .iter()
        .filter(|r| r.method == "GET" && r.range.is_none())
        .count();
    assert_eq!(gets, 1, "exactly one body request");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mid_stream_disconnect_reconnects_and_completes() {
    let server = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 1024 * 1024;
    let run = Run::new(test_settings());
    // `throttle` makes the cut genuinely mid-stream (an unthrottled loopback server aborts
    // before the client has parsed the headers, which is a different failure).
    let url = server.url(&format!(
        "/file/fa.bin?size={size}&fail_after=300000&throttle=3000000"
    ));
    let task = http_task(vec![url], "fa.bin", dir.path());
    let outcome = download(task, &run).await;
    dump(&run);
    let (path, bytes) = assert_completed(&outcome);
    assert_eq!(bytes, size);
    assert_file_matches(&path, "fa.bin", size);
    assert!(run.sink.has_stat(|s| matches!(s, EngineStat::Retry)));
    assert!(run.sink.has_log("segment.retry"));
    let resumed = server
        .requests()
        .iter()
        .filter(|r| r.method == "GET")
        .filter_map(|r| r.range.as_deref().and_then(range_start))
        .any(|s| s > 0);
    assert!(resumed, "reconnect must continue from the written offset");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transient_503s_are_retried() {
    let server = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 256 * 1024;
    let run = Run::new(test_settings());
    let url = server.url(&format!("/file/ff.bin?size={size}&fail_first=2"));
    let task = http_task(vec![url], "ff.bin", dir.path());
    let (path, _) = assert_completed(&download(task, &run).await);
    assert_file_matches(&path, "ff.bin", size);
    assert!(run.sink.has_stat(|s| matches!(s, EngineStat::Retry)));
    assert!(server.requests_for("ff.bin") >= 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn not_found_is_permanent() {
    let server = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let run = Run::new(test_settings());
    let url = server.url("/file/missing.bin?status=404");
    let task = http_task(vec![url], "missing.bin", dir.path());
    let e = assert_failed(&download(task, &run).await, ErrorKind::NotFound);
    assert_eq!(e.class(), FailureClass::Permanent);
    assert_eq!(e.status_code, Some(404));
    assert!(!dir.path().join("missing.bin").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn throttling_429_shrinks_connections_and_completes() {
    let server = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 4 * 1024 * 1024;
    let run = Run::new(test_settings());
    let url = server.url(&format!(
        "/file/tm.bin?size={size}&max_conn=2&throttle=3000000"
    ));
    let task = http_task(vec![url], "tm.bin", dir.path());
    let (path, _) = assert_completed(&download(task, &run).await);
    assert_file_matches(&path, "tm.bin", size);
    assert!(run.sink.has_stat(|s| matches!(s, EngineStat::Throttled)));
    assert!(run.sink.has_log("adaptive.shrink"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_then_resume_from_checkpoint() {
    let server = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 4 * 1024 * 1024;
    let url = server.url(&format!("/file/pr.bin?size={size}&throttle=800000"));

    // --- first run: pause after ~1 MB ---
    let run1 = Run::new(test_settings());
    let control = run1.control.clone();
    let handle = {
        let ctx = run1.ctx(http_task(vec![url.clone()], "pr.bin", dir.path()));
        tokio::spawn(async move { engine().run(ctx).await })
    };
    let started = Instant::now();
    while control.counters.downloaded() < 1024 * 1024 {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "download stalled"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let pause_at = Instant::now();
    control.pause.cancel();
    let outcome = handle.await.unwrap();
    assert!(
        pause_at.elapsed() < Duration::from_secs(2),
        "pause must be prompt"
    );
    assert!(matches!(outcome, TransferOutcome::Paused), "{outcome:?}");
    let checkpoint = run1.sink.last_checkpoint().expect("checkpoint on pause");
    let map = checkpoint.as_segments().unwrap().clone();
    assert!(
        map.committed_bytes() >= 512 * 1024,
        "{}",
        map.committed_bytes()
    );
    assert!(!map.is_complete());
    assert_eq!(map.total, Some(size));
    assert!(map.etag.is_some());
    let part = dir.path().join("pr.bin.osprey-part");
    assert!(part.exists(), "part file kept on pause");
    let requests_before = server.requests().len();

    // --- second run: resume ---
    let mut run2 = Run::new(test_settings());
    run2.checkpoint = Some(checkpoint);
    let (path, bytes) =
        assert_completed(&download(http_task(vec![url], "pr.bin", dir.path()), &run2).await);
    assert_eq!(bytes, size);
    assert_file_matches(&path, "pr.bin", size);
    assert!(run2.sink.has_log("http.resume"));
    // every data request after the resume starts at or beyond the committed watermark
    let after: Vec<_> = server
        .requests()
        .into_iter()
        .skip(requests_before)
        .collect();
    let mut data_requests = 0;
    for r in after.iter().filter(|r| r.method == "GET") {
        let Some(range) = r.range.as_deref() else {
            continue;
        };
        if range == "bytes=0-0" {
            continue; // probe
        }
        data_requests += 1;
        let start = range_start(range).unwrap();
        let seg = map
            .segments
            .iter()
            .find(|s| s.start <= start && start < s.end.max(s.start + 1))
            .unwrap_or_else(|| panic!("no segment for offset {start}"));
        assert!(
            start >= seg.committed,
            "range {range} starts before committed {} of segment {}",
            seg.committed,
            seg.index
        );
    }
    assert!(data_requests > 0);
    // resume did not re-download everything
    let downloaded_after_resume = run2.control.counters.downloaded();
    assert_eq!(downloaded_after_resume, size);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn etag_change_between_pause_and_resume_is_source_changed() {
    let server = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 4 * 1024 * 1024;
    let url = server.url(&format!("/file/et.bin?size={size}&throttle=800000"));
    let run1 = Run::new(test_settings());
    let control = run1.control.clone();
    let handle = {
        let ctx = run1.ctx(http_task(vec![url.clone()], "et.bin", dir.path()));
        tokio::spawn(async move { engine().run(ctx).await })
    };
    let started = Instant::now();
    while control.counters.downloaded() < 512 * 1024 {
        assert!(started.elapsed() < Duration::from_secs(20));
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    control.pause.cancel();
    assert!(matches!(handle.await.unwrap(), TransferOutcome::Paused));
    let checkpoint = run1.sink.last_checkpoint().unwrap();

    let mut run2 = Run::new(test_settings());
    run2.checkpoint = Some(checkpoint);
    let changed = format!("{url}&etag=%22changed%22");
    let e = assert_failed(
        &download(http_task(vec![changed], "et.bin", dir.path()), &run2).await,
        ErrorKind::SourceChanged,
    );
    assert_eq!(e.class(), FailureClass::RestartFromScratch);
    assert!(e.detail.as_deref().unwrap_or("").contains("etag"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn redirects_are_followed() {
    let server = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 512 * 1024;
    let run = Run::new(test_settings());
    let url = server.url(&format!("/file/rd.bin?size={size}&redirect=3"));
    let task = http_task(vec![url], "rd.bin", dir.path());
    let (path, _) = assert_completed(&download(task, &run).await);
    assert_file_matches(&path, "rd.bin", size);
    let meta = run.sink.metadata.lock().clone().unwrap();
    assert!(meta.final_url.unwrap().contains("redirect=0"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn basic_auth_from_secrets() {
    let server = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 256 * 1024;
    let url = server.url(&format!("/file/au.bin?size={size}&auth=user:pass"));

    let run = Run::new(test_settings());
    let e = assert_failed(
        &download(http_task(vec![url.clone()], "au.bin", dir.path()), &run).await,
        ErrorKind::AuthenticationRequired,
    );
    assert_eq!(e.class(), FailureClass::NeedsUser);

    let mut run = Run::new(test_settings());
    run.secrets.username = Some("user".into());
    run.secrets.password = Some("pass".into());
    let (path, _) =
        assert_completed(&download(http_task(vec![url], "au.bin", dir.path()), &run).await);
    assert_file_matches(&path, "au.bin", size);
    assert!(server.requests().iter().any(|r| r.authorization));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn content_disposition_names_the_file() {
    let server = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 128 * 1024;
    let run = Run::new(test_settings());
    let url = server.url(&format!(
        "/file/cd.bin?size={size}&disposition=report%202025.pdf"
    ));
    let task = http_task(vec![url], "cd.bin", dir.path());
    let (path, _) = assert_completed(&download(task, &run).await);
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        "report 2025.pdf"
    );
    assert_file_matches(&path, "cd.bin", size);
    let meta = run.sink.metadata.lock().clone().unwrap();
    assert_eq!(meta.name.as_deref(), Some("report 2025.pdf"));
    assert!(meta.content_disposition.is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn locked_name_wins_over_disposition() {
    let server = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 64 * 1024;
    let run = Run::new(test_settings());
    let url = server.url(&format!("/file/ln.bin?size={size}&disposition=other.bin"));
    let mut task = http_task(vec![url], "mine.bin", dir.path());
    task.name_locked = true;
    let (path, _) = assert_completed(&download(task, &run).await);
    assert_eq!(path, dir.path().join("mine.bin"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn metalink_hash_mismatch_is_detected() {
    let server = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 512 * 1024;
    let good_hash = sha256_hex(&content_for("ml.bin", size));
    let url = server
        .url(&format!("/file/ml.bin?size={size}&corrupt=1"))
        .replace('&', "&amp;");
    let doc = format!(
        r#"<?xml version="1.0"?><metalink xmlns="urn:ietf:params:xml:ns:metalink"><file name="ml.bin"><size>{size}</size><hash type="sha-256">{good_hash}</hash><url priority="1">{url}</url></file></metalink>"#
    );
    let run = Run::new(test_settings());
    let task = Task::new(
        TaskKind::Metalink,
        Source::Metalink {
            url: None,
            document: Some(doc),
        },
        "ml.bin",
        dir.path().to_path_buf(),
        QueueId::default_queue(),
    );
    let e = assert_failed(&download(task, &run).await, ErrorKind::ChecksumMismatch);
    assert_eq!(e.class(), FailureClass::RestartFromScratch);
    assert!(run.sink.states.lock().contains(&TaskState::Verifying));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn metalink_good_hash_completes_and_multi_file_warns() {
    let server = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 300 * 1024;
    let hash = sha256_hex(&content_for("mg.bin", size));
    let url = server.file_url("mg.bin", size);
    let doc = format!(
        r#"<metalink version="3.0" xmlns="http://www.metalinker.org/"><files><file name="mg.bin"><size>{size}</size><verification><hash type="sha256">{hash}</hash></verification><resources><url type="http" preference="100">{url}</url></resources></file><file name="second.bin"><resources><url type="http">{url}</url></resources></file></files></metalink>"#
    );
    // fetched from a URL rather than inline
    server.add_text("meta.metalink", "application/metalink+xml", &doc);
    let run = Run::new(test_settings());
    let task = Task::new(
        TaskKind::Metalink,
        Source::Metalink {
            url: Some(server.url("/text/meta.metalink")),
            document: None,
        },
        "",
        dir.path().to_path_buf(),
        QueueId::default_queue(),
    );
    let (path, _) = assert_completed(&download(task, &run).await);
    assert_eq!(path, dir.path().join("mg.bin"));
    assert_file_matches(&path, "mg.bin", size);
    assert!(run.sink.has_log("metalink.multi_file"));
    assert!(run.sink.has_log("http.verify"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mirrors_complete_via_the_healthy_one() {
    let server = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 2 * 1024 * 1024;
    let bad = server.url(&format!("/file/mr.bin?size={size}&status=503"));
    let good = server.file_url("mr.bin", size);
    let run = Run::new(test_settings());
    let task = http_task(vec![bad, good], "mr.bin", dir.path());
    let (path, _) = assert_completed(&download(task, &run).await);
    assert_file_matches(&path, "mr.bin", size);
    assert!(run.sink.has_log("mirror.switch"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mirror_failover_mid_download() {
    let server = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 4 * 1024 * 1024;
    // the flaky mirror drops every connection after 100 KB; the other one is fine
    let flaky = server.url(&format!("/file/mf.bin?size={size}&fail_after=100000"));
    let good = server.file_url("mf.bin", size);
    let run = Run::new(test_settings());
    let task = http_task(vec![flaky, good], "mf.bin", dir.path());
    let (path, _) = assert_completed(&download(task, &run).await);
    assert_file_matches(&path, "mf.bin", size);
    assert!(run
        .sink
        .has_stat(|s| matches!(s, EngineStat::MirrorSwitched { .. })));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn all_mirrors_failing_is_mirror_exhausted() {
    let server = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let a = server.url("/file/mx.bin?size=1000&status=503");
    let b = server.url("/file/mx.bin?size=1000&status=500");
    let run = Run::new(test_settings());
    let task = http_task(vec![a, b], "mx.bin", dir.path());
    let outcome = download(task, &run).await;
    match outcome {
        TransferOutcome::Failed(e) => assert!(
            matches!(e.kind, ErrorKind::MirrorExhausted | ErrorKind::ServerError),
            "{e:?}"
        ),
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chunked_unknown_length_completes() {
    let server = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 700 * 1024;

    // HEAD says `Content-Length: 0` (no HEAD body); the ranged GET reveals the size.
    let run = Run::new(test_settings());
    let url = server.url(&format!("/file/nl.bin?size={size}&nolength=1"));
    let task = http_task(vec![url], "nl.bin", dir.path());
    let outcome = download(task, &run).await;
    dump(&run);
    let (path, bytes) = assert_completed(&outcome);
    assert_eq!(bytes, size);
    assert_file_matches(&path, "nl.bin", size);
    let meta = run.sink.metadata.lock().clone().unwrap();
    assert_eq!(meta.total, Some(size));

    // No ranges either: the size stays unknown until EOF.
    let run = Run::new(test_settings());
    let url = server.url(&format!("/file/nl2.bin?size={size}&nolength=1&norange=1"));
    let task = http_task(vec![url], "nl2.bin", dir.path());
    let outcome = download(task, &run).await;
    dump(&run);
    let (path, bytes) = assert_completed(&outcome);
    assert_eq!(bytes, size);
    assert_file_matches(&path, "nl2.bin", size);
    let meta = run.sink.metadata.lock().clone().unwrap();
    assert_eq!(meta.total, None);
    assert!(run.sink.progress.lock().iter().all(|p| p.total.is_none()));
    assert!(run
        .sink
        .has_stat(|s| matches!(s, EngineStat::RangeSupport { supported: false })));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_is_prompt_and_keeps_part_file() {
    let server = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 4 * 1024 * 1024;
    let url = server.url(&format!("/file/cn.bin?size={size}&throttle=500000"));
    let run = Run::new(test_settings());
    let control = run.control.clone();
    let handle = {
        let ctx = run.ctx(http_task(vec![url], "cn.bin", dir.path()));
        tokio::spawn(async move { engine().run(ctx).await })
    };
    let started = Instant::now();
    while control.counters.downloaded() < 64 * 1024 {
        assert!(started.elapsed() < Duration::from_secs(20));
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let cancel_at = Instant::now();
    control.cancel.cancel();
    let outcome = handle.await.unwrap();
    assert!(
        cancel_at.elapsed() < Duration::from_secs(1),
        "{:?}",
        cancel_at.elapsed()
    );
    assert!(matches!(outcome, TransferOutcome::Cancelled), "{outcome:?}");
    assert!(dir.path().join("cn.bin.osprey-part").exists());
    assert!(!dir.path().join("cn.bin").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rate_limiter_caps_throughput() {
    let server = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 300 * 1024;
    let mut run = Run::new(test_settings());
    run.limiter = RateLimiter::new("limited", 100_000, None);
    run.limiter.set_burst(16 * 1024);
    let url = server.url(&format!("/file/rl.bin?size={size}&throttle=200000"));
    let started = Instant::now();
    let (path, _) =
        assert_completed(&download(http_task(vec![url], "rl.bin", dir.path()), &run).await);
    let rate = size as f64 / started.elapsed().as_secs_f64();
    assert!(rate <= 120_000.0, "rate {rate:.0} B/s exceeds the limiter");
    assert_file_matches(&path, "rl.bin", size);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unsupported_scheme_and_html_interstitial() {
    let server = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let run = Run::new(test_settings());
    let e = assert_failed(
        &download(
            http_task(vec!["file:///etc/passwd".into()], "passwd", dir.path()),
            &run,
        )
        .await,
        ErrorKind::UnsupportedScheme,
    );
    assert_eq!(e.class(), FailureClass::Permanent);

    let url = server.url("/file/page.bin?size=2048&ctype=text/html");
    let run = Run::new(test_settings());
    assert_failed(
        &download(http_task(vec![url], "page.bin", dir.path()), &run).await,
        ErrorKind::UnexpectedContent,
    );
    // …but a probe of the same URL succeeds and reports the mime
    let settings = Arc::new(test_settings());
    let meta = HttpEngine::probe_url(
        &server.url("/file/page.bin?size=2048&ctype=text/html"),
        settings.clone(),
        ClientFactory::new(settings),
        &TransferSecrets::default(),
    )
    .await
    .unwrap();
    assert_eq!(meta.mime.as_deref(), Some("text/html"));
    assert_eq!(meta.total, Some(2048));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn probe_url_reports_metadata() {
    let server = TestServer::start().await;
    let settings = Arc::new(test_settings());
    let meta = HttpEngine::probe_url(
        &server.url("/file/p.bin?size=4096&disposition=x.iso"),
        settings.clone(),
        ClientFactory::new(settings),
        &TransferSecrets::default(),
    )
    .await
    .unwrap();
    assert_eq!(meta.name.as_deref(), Some("x.iso"));
    assert_eq!(meta.total, Some(4096));
    assert_eq!(meta.resumable, Some(true));
    assert!(meta.etag.is_some());
    assert!(meta.last_modified.is_some());
    assert_eq!(meta.mime.as_deref(), Some("application/octet-stream"));
    // HEAD only: no body was fetched
    assert!(server.requests().iter().all(|r| r.method == "HEAD"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn existing_final_file_gets_a_unique_name() {
    let server = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("u.bin"), b"old").unwrap();
    let run = Run::new(test_settings());
    let task = http_task(vec![server.file_url("u.bin", 4096)], "u.bin", dir.path());
    let (path, _) = assert_completed(&download(task, &run).await);
    assert_eq!(path.file_name().unwrap().to_str().unwrap(), "u (2).bin");
    assert_eq!(std::fs::read(dir.path().join("u.bin")).unwrap(), b"old");
    assert_file_matches(&path, "u.bin", 4096);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn user_headers_and_referer_are_sent() {
    let server = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let run = Run::new(test_settings());
    let mut task = http_task(vec![server.file_url("h.bin", 4096)], "h.bin", dir.path());
    task.options.referer = Some("https://example.com/page".into());
    task.options.user_agent = Some("OspreyTest/1.0".into());
    assert_completed(&download(task, &run).await);
    let reqs = server.requests();
    assert!(reqs
        .iter()
        .all(|r| r.referer.as_deref() == Some("https://example.com/page")));
    assert!(reqs
        .iter()
        .all(|r| r.user_agent.as_deref() == Some("OspreyTest/1.0")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn adaptive_controller_grows_when_throughput_scales() {
    let server = TestServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 24 * 1024 * 1024;
    // Each connection is paced at 4 MB/s, so every extra connection adds throughput and the
    // growth experiments must be kept.
    let mut settings = test_settings();
    settings.network.connections_per_task = 2;
    let run = Run::new(settings);
    let url = server.url(&format!("/file/ag.bin?size={size}&throttle=4000000"));
    let task = http_task(vec![url], "ag.bin", dir.path());
    let outcome = download(task, &run).await;
    dump(&run);
    let (path, _) = assert_completed(&outcome);
    assert_file_matches(&path, "ag.bin", size);
    assert!(
        run.sink.has_log("adaptive.grow"),
        "controller never tried to grow"
    );
    assert!(run
        .sink
        .has_stat(|s| matches!(s, EngineStat::SegmentReassigned)));
    let max_active = run
        .sink
        .progress
        .lock()
        .iter()
        .map(|p| p.active_connections)
        .max()
        .unwrap_or(0);
    assert!(
        max_active > 2,
        "connections never exceeded the initial 2 (max {max_active})"
    );
    let ranged = server
        .requests()
        .iter()
        .filter(|r| r.method == "GET" && r.range.is_some())
        .count();
    assert!(
        ranged > 3,
        "expected work-stealing splits, saw {ranged} range requests"
    );
}
