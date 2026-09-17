use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
use bytes::Bytes;
use osprey_domain::checkpoint::Checkpoint;
use osprey_domain::settings::Settings;
use osprey_domain::{ErrorKind, Millis, Progress, QueueId, Source, Task, TaskKind, TaskState};
use osprey_media::HlsEngine;
use osprey_runtime::engine::{
    EngineStat, ProgressSink, ResolvedMetadata, Transfer, TransferContext, TransferControl,
    TransferOutcome, TransferSecrets,
};
use osprey_runtime::net::ClientFactory;
use osprey_runtime::RateLimiter;
use osprey_testserver::{content_for, TestServer};
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

struct Sink {
    checkpoints: Mutex<Vec<Checkpoint>>,
    states: Mutex<Vec<TaskState>>,
    progress: Mutex<Vec<Progress>>,
}
impl Sink {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            checkpoints: Mutex::new(vec![]),
            states: Mutex::new(vec![]),
            progress: Mutex::new(vec![]),
        })
    }
}
impl ProgressSink for Sink {
    fn progress(&self, p: Progress) {
        self.progress.lock().push(p);
    }
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

fn ctx(
    task: Task,
    sink: Arc<Sink>,
    checkpoint: Option<Checkpoint>,
) -> (TransferContext, Arc<TransferControl>) {
    let mut settings = Settings::default();
    settings.storage.quarantine_downloads = false;
    settings.network.connections_per_task = 4;
    let settings = Arc::new(settings);
    let control = TransferControl::new();
    let global = RateLimiter::unlimited("global");
    let c = TransferContext {
        task,
        checkpoint,
        control: control.clone(),
        sink,
        settings: settings.clone(),
        secrets: TransferSecrets::default(),
        limiter: global.child("task", 0),
        upload_limiter: RateLimiter::unlimited("up"),
        clients: ClientFactory::new(settings),
        started_at: Millis::now(),
    };
    (c, control)
}

fn task(dir: &std::path::Path, playlist: String, variant: Option<String>) -> Task {
    Task::new(
        TaskKind::Hls,
        Source::Hls {
            playlist_url: playlist,
            variant,
        },
        "movie.ts",
        dir.to_path_buf(),
        QueueId::default_queue(),
    )
}

/// Register a TS playlist with `n` segments of `seg_size` bytes; returns expected merged bytes.
fn register_ts(server: &TestServer, prefix: &str, n: u32, seg_size: u64) -> Vec<u8> {
    let mut expected = Vec::new();
    let mut pl = String::from(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:0\n",
    );
    for i in 0..n {
        let body = content_for(&format!("{prefix}-{i}"), seg_size);
        expected.extend_from_slice(&body);
        server.add_bytes(&format!("{prefix}/seg{i}.ts"), "video/mp2t", body);
        pl.push_str(&format!(
            "#EXTINF:4.0,\n{}\n",
            server.url(&format!("/bytes/{prefix}/seg{i}.ts"))
        ));
    }
    pl.push_str("#EXT-X-ENDLIST\n");
    server.add_text(
        &format!("{prefix}/index.m3u8"),
        "application/vnd.apple.mpegurl",
        &pl,
    );
    expected
}

#[tokio::test]
async fn downloads_and_merges_ts() {
    let server = TestServer::start().await;
    let expected = register_ts(&server, "vod", 12, 50_000);
    let master = format!("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\n{}\n#EXT-X-STREAM-INF:BANDWIDTH=200000,RESOLUTION=320x180\n{}\n", server.url("/text/vod/index.m3u8"), server.url("/text/low/index.m3u8"));
    register_ts(&server, "low", 3, 1000);
    server.add_text("master.m3u8", "application/vnd.apple.mpegurl", &master);
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    let sink = Sink::new();
    let (c, _control) = ctx(
        task(&out, server.url("/text/master.m3u8"), None),
        sink.clone(),
        None,
    );
    let engine = HlsEngine::new();
    match engine.run(c).await {
        TransferOutcome::Completed { file_path, bytes } => {
            // highest bandwidth variant chosen; ffmpeg may have remuxed to mp4
            if file_path.extension().and_then(|e| e.to_str()) == Some("ts") {
                assert_eq!(std::fs::read(&file_path).unwrap(), expected);
                assert_eq!(bytes, expected.len() as u64);
            } else {
                assert!(bytes > 0);
            }
            assert!(!out.join("movie.ts.osprey-part").exists());
            assert!(file_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap()
                .starts_with("movie"));
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(sink.states.lock().contains(&TaskState::Processing));
    assert!(sink
        .checkpoints
        .lock()
        .iter()
        .any(|c| matches!(c, Checkpoint::Hls(h) if h.done_count() == 12)));
    // twelve segments were fetched, none twice
    assert_eq!(
        server
            .requests()
            .iter()
            .filter(|r| r.path.contains("/bytes/vod/seg"))
            .count(),
        12
    );
}

#[tokio::test]
async fn aes128_fmp4_with_byte_ranges_and_explicit_variant() {
    let server = TestServer::start().await;
    let key = [7u8; 16];
    server.add_bytes(
        "k/key.bin",
        "application/octet-stream",
        Bytes::copy_from_slice(&key),
    );
    let init = content_for("init", 700);
    server.add_bytes("enc/init.mp4", "video/mp4", init.clone());
    // one big file holding 4 encrypted segments back-to-back, addressed by byte range
    let mut blob = Vec::new();
    let mut expected = init.to_vec();
    let mut pl = format!("#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:5\n#EXT-X-MAP:URI=\"{}\"\n#EXT-X-KEY:METHOD=AES-128,URI=\"{}\"\n", server.url("/bytes/enc/init.mp4"), server.url("/bytes/k/key.bin"));
    for i in 0..4u64 {
        let plain = content_for(&format!("p{i}"), 3000 + i * 17);
        expected.extend_from_slice(&plain);
        let mut iv = [0u8; 16];
        iv[8..].copy_from_slice(&(5 + i).to_be_bytes());
        let enc = cbc::Encryptor::<aes::Aes128>::new(&key.into(), &iv.into())
            .encrypt_padded_vec_mut::<Pkcs7>(&plain);
        pl.push_str(&format!(
            "#EXTINF:2.0,\n#EXT-X-BYTERANGE:{}@{}\n{}\n",
            enc.len(),
            blob.len(),
            server.url("/bytes/enc/all.m4s")
        ));
        blob.extend_from_slice(&enc);
    }
    pl.push_str("#EXT-X-ENDLIST\n");
    server.add_bytes("enc/all.m4s", "video/iso.segment", Bytes::from(blob));
    server.add_text("enc/index.m3u8", "application/vnd.apple.mpegurl", &pl);
    let master = format!("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=5000000,NAME=\"hd\"\n{}\n#EXT-X-STREAM-INF:BANDWIDTH=100,NAME=\"tiny\"\n{}\n", server.url("/text/missing.m3u8"), server.url("/text/enc/index.m3u8"));
    server.add_text("m2.m3u8", "application/vnd.apple.mpegurl", &master);
    let dir = tempfile::tempdir().unwrap();
    let sink = Sink::new();
    let (c, _control) = ctx(
        task(dir.path(), server.url("/text/m2.m3u8"), Some("tiny".into())),
        sink.clone(),
        None,
    );
    match HlsEngine::new().run(c).await {
        TransferOutcome::Completed { file_path, .. } => {
            assert_eq!(file_path.extension().and_then(|e| e.to_str()), Some("mp4"));
            assert_eq!(std::fs::read(&file_path).unwrap(), expected);
        }
        other => panic!("unexpected {other:?}"),
    }
    // key fetched exactly once
    assert_eq!(
        server
            .requests()
            .iter()
            .filter(|r| r.path.ends_with("k/key.bin"))
            .count(),
        1
    );
    // key cached in checkpoint
    assert!(sink
        .checkpoints
        .lock()
        .iter()
        .any(|c| matches!(c, Checkpoint::Hls(h) if !h.key_cache.is_empty())));
}

#[tokio::test]
async fn resumes_from_checkpoint_without_refetching() {
    let server = TestServer::start().await;
    let expected = register_ts(&server, "res", 8, 20_000);
    let dir = tempfile::tempdir().unwrap();
    let sink = Sink::new();
    // first run: complete it, then reconstruct a partial checkpoint from what it produced
    let (c, _control) = ctx(
        task(dir.path(), server.url("/text/res/index.m3u8"), None),
        sink.clone(),
        None,
    );
    let engine = HlsEngine::new();
    let first = engine.run(c).await;
    assert!(matches!(first, TransferOutcome::Completed { .. }));
    let requests_after_first = server
        .requests()
        .iter()
        .filter(|r| r.path.contains("/bytes/res/seg"))
        .count();
    assert_eq!(requests_after_first, 8);

    // Simulate an interrupted run: recreate the part dir with segments 0..5 present and a
    // checkpoint claiming 0..5 done (plus a stale claim for 6 whose file is missing).
    let part = dir.path().join("movie.ts.osprey-part");
    std::fs::create_dir_all(&part).unwrap();
    let mut cp = osprey_domain::checkpoint::HlsCheckpoint {
        media_playlist_url: server.url("/text/res/index.m3u8"),
        part_dir: part.clone(),
        segment_count: 8,
        container: "ts".into(),
        ..Default::default()
    };
    for i in 0..6u32 {
        std::fs::write(
            part.join(format!("seg-{i:06}.bin")),
            content_for(&format!("res-{i}"), 20_000),
        )
        .unwrap();
        cp.set_done(i);
    }
    cp.set_done(6); // stale: file missing
    let t = task(dir.path(), server.url("/text/res/index.m3u8"), None);
    let sink2 = Sink::new();
    let (c, _control) = ctx(t, sink2.clone(), Some(Checkpoint::Hls(cp)));
    match engine.run(c).await {
        TransferOutcome::Completed { file_path, .. } => {
            if file_path.extension().and_then(|e| e.to_str()) == Some("ts") {
                assert_eq!(std::fs::read(&file_path).unwrap(), expected);
            }
        }
        other => panic!("unexpected {other:?}"),
    }
    let requests_after_second = server
        .requests()
        .iter()
        .filter(|r| r.path.contains("/bytes/res/seg"))
        .count();
    // only segments 6 and 7 were fetched again
    assert_eq!(requests_after_second - requests_after_first, 2);
}

#[tokio::test]
async fn refuses_drm_and_live() {
    let server = TestServer::start().await;
    server.add_text("drm.m3u8", "application/vnd.apple.mpegurl", "#EXTM3U\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://x\",KEYFORMAT=\"com.apple.streamingkeydelivery\"\n#EXTINF:4,\na.ts\n#EXT-X-ENDLIST\n");
    server.add_text(
        "live.m3u8",
        "application/vnd.apple.mpegurl",
        "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\na.ts\n",
    );
    let dir = tempfile::tempdir().unwrap();
    let engine = HlsEngine::new();
    let (c, _) = ctx(
        task(dir.path(), server.url("/text/drm.m3u8"), None),
        Sink::new(),
        None,
    );
    match engine.run(c).await {
        TransferOutcome::Failed(e) => assert_eq!(e.kind, ErrorKind::ProtectedContent),
        other => panic!("unexpected {other:?}"),
    }
    let (c, _) = ctx(
        task(dir.path(), server.url("/text/live.m3u8"), None),
        Sink::new(),
        None,
    );
    match engine.run(c).await {
        TransferOutcome::Failed(e) => assert_eq!(e.kind, ErrorKind::LiveStreamUnsupported),
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn pause_returns_promptly_with_checkpoint() {
    let server = TestServer::start().await;
    // slow segments so we can pause mid-way
    let mut pl = String::from("#EXTM3U\n#EXT-X-TARGETDURATION:4\n");
    for i in 0..6 {
        pl.push_str(&format!(
            "#EXTINF:4,\n{}\n",
            server.url(&format!("/file/slow{i}?size=400000&throttle=200000"))
        ));
    }
    pl.push_str("#EXT-X-ENDLIST\n");
    server.add_text("slow.m3u8", "application/vnd.apple.mpegurl", &pl);
    let dir = tempfile::tempdir().unwrap();
    let sink = Sink::new();
    let (c, control) = ctx(
        task(dir.path(), server.url("/text/slow.m3u8"), None),
        sink.clone(),
        None,
    );
    let engine = HlsEngine::new();
    let run = tokio::spawn(async move { engine.run(c).await });
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let t0 = std::time::Instant::now();
    control.pause.cancel();
    let outcome = run.await.unwrap();
    assert!(
        t0.elapsed() < Duration::from_secs(2),
        "pause took {:?}",
        t0.elapsed()
    );
    assert_eq!(outcome, TransferOutcome::Paused);
    let part = dir.path().join("movie.ts.osprey-part");
    assert!(part.exists(), "part dir kept for resume");
    assert!(!sink.checkpoints.lock().is_empty());
    let _ = PathBuf::new();
}

#[tokio::test]
async fn detect_describes_variants() {
    let server = TestServer::start().await;
    register_ts(&server, "d", 3, 100);
    let master = format!("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=3000000,RESOLUTION=1920x1080,CODECS=\"avc1.64002a,mp4a.40.2\"\n{}\n", server.url("/text/d/index.m3u8"));
    server.add_text("dm.m3u8", "application/vnd.apple.mpegurl", &master);
    let clients = ClientFactory::new(Arc::new(Settings::default()));
    let d = osprey_media::detect::detect(&server.url("/text/dm.m3u8"), None, clients)
        .await
        .unwrap();
    assert_eq!(d.kind, osprey_domain::media::MediaKind::HlsPlaylist);
    assert_eq!(d.variants[0].height, Some(1080));
    assert!(d.variants[0].label.starts_with("1080p"));
    assert_eq!(
        d.variants[0].estimated_size,
        Some((12.0 * 3_000_000.0 / 8.0) as u64)
    );
    assert!(!d.protected);
}
