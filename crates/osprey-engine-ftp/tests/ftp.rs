mod support;

use osprey_domain::checkpoint::Checkpoint;
use osprey_domain::settings::Settings;
use osprey_domain::{ErrorKind, Millis, Progress, QueueId, Source, Task, TaskKind, TaskState};
use osprey_engine_ftp::FtpEngine;
use osprey_runtime::engine::{
    EngineStat, ProgressSink, ResolvedMetadata, Transfer, TransferContext, TransferControl,
    TransferOutcome, TransferSecrets,
};
use osprey_runtime::net::ClientFactory;
use osprey_runtime::RateLimiter;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use support::ftpd::FtpServer;

struct Sink {
    checkpoints: Mutex<Vec<Checkpoint>>,
    states: Mutex<Vec<TaskState>>,
}
impl Sink {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            checkpoints: Mutex::new(vec![]),
            states: Mutex::new(vec![]),
        })
    }
}
impl ProgressSink for Sink {
    fn progress(&self, _: Progress) {}
    fn state(&self, s: TaskState, _: Option<String>) {
        self.states.lock().push(s);
    }
    fn metadata(&self, _: ResolvedMetadata) {}
    fn checkpoint(&self, c: Checkpoint) {
        self.checkpoints.lock().push(c);
    }
    fn log(&self, _: osprey_domain::events::LogLevel, _: &str, _: String) {}
    fn stat(&self, _: EngineStat) {}
}

fn body(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 7 % 251) as u8).collect()
}

fn ctx(
    url: &str,
    dir: &std::path::Path,
    sink: Arc<Sink>,
    checkpoint: Option<Checkpoint>,
    limit: u64,
    secrets: TransferSecrets,
) -> (TransferContext, Arc<TransferControl>) {
    let mut settings = Settings::default();
    settings.storage.quarantine_downloads = false;
    settings.network.retry_base_delay_ms = 100;
    let settings = Arc::new(settings);
    let task = Task::new(
        TaskKind::Ftp,
        Source::Urls {
            urls: vec![url.to_owned()],
        },
        "file.bin",
        dir.to_path_buf(),
        QueueId::default_queue(),
    );
    let control = TransferControl::new();
    let global = RateLimiter::new("global", limit, None);
    let c = TransferContext {
        task,
        checkpoint,
        control: control.clone(),
        sink,
        settings: settings.clone(),
        secrets,
        limiter: global.child("task", 0),
        upload_limiter: RateLimiter::unlimited("up"),
        clients: ClientFactory::new(settings),
        started_at: Millis::now(),
    };
    (c, control)
}

#[tokio::test]
async fn probe_and_full_download() {
    let data = body(300_000);
    let server = FtpServer::start(
        HashMap::from([("/pub/file.bin".to_owned(), data.clone())]),
        None,
        false,
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let engine = FtpEngine::new();
    let url = server.url("/pub/file.bin", Some(("u", "p")));
    let (c, _) = ctx(
        &url,
        dir.path(),
        Sink::new(),
        None,
        0,
        TransferSecrets::default(),
    );
    let meta = engine
        .probe(
            &c.task,
            c.settings.clone(),
            TransferSecrets::default(),
            c.clients.clone(),
        )
        .await
        .unwrap();
    assert_eq!(meta.total, Some(300_000));
    assert_eq!(meta.resumable, Some(true));
    assert_eq!(meta.name.as_deref(), Some("file.bin"));
    assert!(
        !meta.final_url.unwrap().contains(":p@"),
        "password redacted"
    );
    match engine.run(c).await {
        TransferOutcome::Completed { file_path, bytes } => {
            assert_eq!(bytes, 300_000);
            assert_eq!(std::fs::read(file_path).unwrap(), data);
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn reconnects_and_resumes_after_disconnect() {
    let data = body(500_000);
    let server = FtpServer::start(
        HashMap::from([("/f.bin".to_owned(), data.clone())]),
        Some(120_000),
        false,
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let sink = Sink::new();
    let (c, _) = ctx(
        &server.url("/f.bin", None),
        dir.path(),
        sink.clone(),
        None,
        0,
        TransferSecrets::default(),
    );
    match FtpEngine::new().run(c).await {
        TransferOutcome::Completed { file_path, .. } => {
            assert_eq!(std::fs::read(file_path).unwrap(), data)
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        server.retr_count.load(std::sync::atomic::Ordering::SeqCst),
        2
    );
    let rests = server.rest_offsets.get();
    assert!(
        rests.iter().any(|&o| o > 0 && o <= 120_000),
        "resumed with REST: {rests:?}"
    );
}

#[tokio::test]
async fn pause_then_resume_from_checkpoint() {
    let data = body(2_000_000);
    let server = FtpServer::start(
        HashMap::from([("/big.bin".to_owned(), data.clone())]),
        None,
        false,
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let sink = Sink::new();
    let (c, control) = ctx(
        &server.url("/big.bin", None),
        dir.path(),
        sink.clone(),
        None,
        400_000,
        TransferSecrets::default(),
    );
    let engine = FtpEngine::new();
    let run = tokio::spawn(async move { engine.run(c).await });
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let t0 = std::time::Instant::now();
    control.pause.cancel();
    let outcome = run.await.unwrap();
    assert!(t0.elapsed() < Duration::from_secs(2));
    assert_eq!(outcome, TransferOutcome::Paused);
    let cp = sink.checkpoints.lock().last().cloned().unwrap();
    let committed = cp.as_segments().unwrap().committed_bytes();
    assert!(
        committed > 0 && committed < 2_000_000,
        "committed {committed}"
    );
    // resume
    let sink2 = Sink::new();
    let (c, _) = ctx(
        &server.url("/big.bin", None),
        dir.path(),
        sink2,
        Some(cp),
        0,
        TransferSecrets::default(),
    );
    match FtpEngine::new().run(c).await {
        TransferOutcome::Completed { file_path, .. } => {
            assert_eq!(std::fs::read(file_path).unwrap(), data)
        }
        other => panic!("{other:?}"),
    }
    let rests = server.rest_offsets.get();
    assert!(
        rests.iter().any(|&o| o == committed),
        "resumed at committed offset: {rests:?}"
    );
}

#[tokio::test]
async fn errors_are_classified() {
    let server = FtpServer::start(
        HashMap::from([("/f.bin".to_owned(), body(10))]),
        None,
        false,
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let (c, _) = ctx(
        &server.url("/f.bin", Some(("u", "wrong"))),
        dir.path(),
        Sink::new(),
        None,
        0,
        TransferSecrets::default(),
    );
    match FtpEngine::new().run(c).await {
        TransferOutcome::Failed(e) => assert_eq!(e.kind, ErrorKind::AuthenticationRequired),
        other => panic!("{other:?}"),
    }
    let (c, _) = ctx(
        &server.url("/missing.bin", None),
        dir.path(),
        Sink::new(),
        None,
        0,
        TransferSecrets::default(),
    );
    match FtpEngine::new().run(c).await {
        TransferOutcome::Failed(e) => assert_eq!(e.kind, ErrorKind::NotFound),
        other => panic!("{other:?}"),
    }
    // secrets supply credentials when the URL has none
    let secrets = TransferSecrets {
        username: Some("u".into()),
        password: Some("p".into()),
        ..Default::default()
    };
    let (c, _) = ctx(
        &server.url("/f.bin", None),
        dir.path(),
        Sink::new(),
        None,
        0,
        secrets,
    );
    assert!(matches!(
        FtpEngine::new().run(c).await,
        TransferOutcome::Completed { .. }
    ));
}

#[tokio::test]
async fn no_rest_support_restarts_from_zero() {
    let data = body(200_000);
    let server = FtpServer::start(
        HashMap::from([("/f.bin".to_owned(), data.clone())]),
        None,
        true,
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("file.bin.osprey-part"), &data[..50_000]).unwrap();
    let mut map = osprey_domain::SegmentMap {
        segments: vec![osprey_domain::Segment::new(0, 0, 200_000)],
        etag: None,
        last_modified: Some("20250101120000".into()),
        total: Some(200_000),
        part_path: None,
    };
    map.segments[0].committed = 50_000;
    let (c, _) = ctx(
        &server.url("/f.bin", None),
        dir.path(),
        Sink::new(),
        Some(Checkpoint::Segments(map)),
        0,
        TransferSecrets::default(),
    );
    match FtpEngine::new().run(c).await {
        TransferOutcome::Completed { file_path, .. } => {
            assert_eq!(std::fs::read(file_path).unwrap(), data)
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn lists_directory_and_respects_rate_limit() {
    let data = body(300_000);
    let server = FtpServer::start(
        HashMap::from([
            ("/a.bin".to_owned(), data.clone()),
            ("/b.txt".to_owned(), body(5)),
        ]),
        None,
        false,
    )
    .await;
    let entries = FtpEngine::list(
        &server.url("/", None),
        &Settings::default(),
        &TransferSecrets::default(),
    )
    .await
    .unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries
        .iter()
        .any(|e| e.name == "a.bin" && e.size == Some(300_000)));
    let dir = tempfile::tempdir().unwrap();
    let (c, _) = ctx(
        &server.url("/a.bin", None),
        dir.path(),
        Sink::new(),
        None,
        150_000,
        TransferSecrets::default(),
    );
    let t0 = std::time::Instant::now();
    assert!(matches!(
        FtpEngine::new().run(c).await,
        TransferOutcome::Completed { .. }
    ));
    let secs = t0.elapsed().as_secs_f64();
    assert!(secs > 1.2, "took {secs}s; limiter not applied");
}
