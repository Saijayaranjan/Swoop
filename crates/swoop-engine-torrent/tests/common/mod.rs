//! Shared fixtures: a librqbit seeder session on loopback, a recording sink, an in-memory blob
//! store and engine/task/context builders. No public internet is touched.

#![allow(dead_code)]

use librqbit::{CreateTorrentOptions, ListenerMode, ListenerOptions, Session, SessionOptions};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use swoop_domain::checkpoint::Checkpoint;
use swoop_domain::settings::Settings;
use swoop_domain::{
    LogLevel, Millis, Progress, QueueId, Source, Task, TaskId, TaskKind, TaskState,
};
use swoop_engine_torrent::{
    SessionTuning, TorrentBlobProvider, TorrentEngine, TorrentEngineConfig,
};
use swoop_runtime::engine::{
    EngineStat, ProgressSink, ResolvedMetadata, TransferContext, TransferControl, TransferSecrets,
};
use swoop_runtime::net::ClientFactory;
use swoop_runtime::RateLimiter;
use tempfile::TempDir;

/// Deterministic pseudo-random content so seeder and expectation agree.
pub fn fill(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1);
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.extend_from_slice(&x.to_le_bytes());
    }
    out.truncate(len);
    out
}

pub struct Seeder {
    pub session: Arc<Session>,
    pub bytes: Vec<u8>,
    pub addr: SocketAddr,
    pub info_hash: String,
    pub name: String,
    pub files: Vec<(String, Vec<u8>)>,
    pub dir: TempDir,
}

/// Start a librqbit session that seeds a freshly created multi-file torrent from `files`.
pub async fn seeder(files: &[(&str, usize)], trackers: Vec<String>) -> Seeder {
    let dir = TempDir::with_prefix("swoop-seed").unwrap();
    let root = dir.path().join("payload");
    std::fs::create_dir_all(&root).unwrap();
    let mut contents = Vec::new();
    for (i, (name, len)) in files.iter().enumerate() {
        let data = fill(i as u64 + 7, *len);
        let path = root.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, &data).unwrap();
        contents.push((name.to_string(), data));
    }
    let session = Session::new_with_opts(
        dir.path().join("unused-output"),
        SessionOptions {
            dht: None,
            disable_trackers: true,
            listen: Some(ListenerOptions {
                mode: ListenerMode::TcpOnly,
                listen_addr: ([127, 0, 0, 1], 0).into(),
                ..Default::default()
            }),
            disable_local_service_discovery: true,
            ..Default::default()
        },
    )
    .await
    .expect("seeder session");
    let (created, handle) = session
        .create_and_serve_torrent(
            &root,
            CreateTorrentOptions {
                name: Some("payload"),
                trackers,
                piece_length: Some(64 * 1024),
            },
        )
        .await
        .expect("create_and_serve_torrent");
    handle.wait_until_initialized().await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while handle.live().is_none() {
        assert!(Instant::now() < deadline, "seeder never went live");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let bytes = created.as_bytes().unwrap().to_vec();
    Seeder {
        addr: session.listen_addr().expect("seeder listen addr"),
        session,
        bytes,
        info_hash: created.info_hash().as_string(),
        name: "payload".into(),
        files: contents,
        dir,
    }
}

#[derive(Default)]
pub struct Recording {
    pub states: Vec<(TaskState, Option<String>)>,
    pub progress: Vec<Progress>,
    pub metadata: Vec<ResolvedMetadata>,
    pub checkpoints: Vec<Checkpoint>,
    pub logs: Vec<(LogLevel, String, String)>,
}

#[derive(Default)]
pub struct RecordingSink(pub Mutex<Recording>);

impl ProgressSink for RecordingSink {
    fn progress(&self, p: Progress) {
        self.0.lock().progress.push(p);
    }
    fn state(&self, s: TaskState, d: Option<String>) {
        self.0.lock().states.push((s, d));
    }
    fn metadata(&self, m: ResolvedMetadata) {
        self.0.lock().metadata.push(m);
    }
    fn checkpoint(&self, c: Checkpoint) {
        self.0.lock().checkpoints.push(c);
    }
    fn log(&self, l: LogLevel, code: &str, m: String) {
        self.0.lock().logs.push((l, code.to_owned(), m));
    }
    fn stat(&self, _: EngineStat) {}
}

impl RecordingSink {
    pub fn states(&self) -> Vec<TaskState> {
        self.0.lock().states.iter().map(|(s, _)| *s).collect()
    }
    pub fn last_progress(&self) -> Option<Progress> {
        self.0.lock().progress.last().cloned()
    }
    pub fn torrent_checkpoints(&self) -> Vec<swoop_domain::checkpoint::TorrentCheckpoint> {
        self.0
            .lock()
            .checkpoints
            .iter()
            .filter_map(|c| c.as_torrent().cloned())
            .collect()
    }
}

pub struct MapBlobs(pub Mutex<HashMap<String, Vec<u8>>>);

#[async_trait::async_trait]
impl TorrentBlobProvider for MapBlobs {
    async fn torrent_bytes(&self, info_hash: &str) -> Option<Vec<u8>> {
        self.0.lock().get(info_hash).cloned()
    }
}

pub fn blobs(entries: &[(&str, &[u8])]) -> Arc<MapBlobs> {
    Arc::new(MapBlobs(Mutex::new(
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_vec()))
            .collect(),
    )))
}

pub fn settings(edit: impl FnOnce(&mut Settings)) -> Arc<Settings> {
    let mut s = Settings::default();
    s.torrent.dht = false;
    s.torrent.listen_port = 0;
    s.torrent.seed_ratio_limit = 0.0;
    edit(&mut s);
    Arc::new(s)
}

pub fn engine(
    base: &Path,
    settings: Arc<Settings>,
    blobs: Arc<MapBlobs>,
    peers: Vec<SocketAddr>,
    trackers: bool,
) -> Arc<TorrentEngine> {
    TorrentEngine::new(TorrentEngineConfig {
        session_dir: base.join("session"),
        default_output: base.join("default-out"),
        settings,
        blobs,
        http: reqwest::Client::new(),
        tuning: SessionTuning {
            disable_trackers: !trackers,
            disable_local_service_discovery: true,
            initial_peers: peers,
            listen_addr: Some("127.0.0.1".parse().unwrap()),
            client_name: Some("Swoop test".into()),
        },
    })
    .expect("engine")
}

pub fn torrent_task(seed: &Seeder, dir: PathBuf) -> Task {
    Task::new(
        TaskKind::Torrent,
        Source::TorrentFile {
            info_hash: seed.info_hash.clone(),
            name: seed.name.clone(),
        },
        seed.name.clone(),
        dir,
        QueueId::default_queue(),
    )
}

pub fn magnet_task(uri: &str, dir: PathBuf) -> Task {
    Task::new(
        TaskKind::Magnet,
        Source::Magnet {
            uri: uri.to_owned(),
        },
        "magnet",
        dir,
        QueueId::default_queue(),
    )
}

pub fn context(
    task: Task,
    settings: Arc<Settings>,
    control: Arc<TransferControl>,
    sink: Arc<RecordingSink>,
    checkpoint: Option<Checkpoint>,
) -> TransferContext {
    TransferContext {
        task,
        checkpoint,
        control,
        sink,
        settings: settings.clone(),
        secrets: TransferSecrets::default(),
        limiter: RateLimiter::unlimited("task"),
        upload_limiter: RateLimiter::unlimited("task-up"),
        clients: ClientFactory::new(settings),
        started_at: Millis::now(),
    }
}

pub fn assert_files_identical(seed: &Seeder, root: &Path) {
    for (name, data) in &seed.files {
        let got = std::fs::read(root.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            got == *data,
            "{name} differs ({} vs {} bytes)",
            got.len(),
            data.len()
        );
    }
}

pub fn task_id(task: &Task) -> TaskId {
    task.id.clone()
}

/// Poll until `f` returns true or the timeout elapses.
pub async fn wait_for(timeout: Duration, mut f: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if f() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    f()
}

/// Minimal BEP-15 tracker: answers connect and announce with fixed swarm numbers and the
/// given peer list.
pub struct UdpStub {
    pub addr: SocketAddr,
    pub announces: Arc<std::sync::atomic::AtomicU32>,
    _task: tokio::task::JoinHandle<()>,
}

pub async fn udp_stub(peers: Vec<SocketAddr>, seeders: u32, leechers: u32) -> UdpStub {
    let sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = sock.local_addr().unwrap();
    let announces = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let counter = announces.clone();
    let task = tokio::spawn(async move {
        let mut buf = vec![0u8; 2048];
        loop {
            let Ok((n, from)) = sock.recv_from(&mut buf).await else {
                return;
            };
            if n < 16 {
                continue;
            }
            let p = &buf[..n];
            let action = u32::from_be_bytes([p[8], p[9], p[10], p[11]]);
            let tid = p[12..16].to_vec();
            let mut reply = Vec::new();
            match action {
                0 => {
                    reply.extend_from_slice(&0u32.to_be_bytes());
                    reply.extend_from_slice(&tid);
                    reply.extend_from_slice(&0x0102_0304_0506_0708u64.to_be_bytes());
                }
                1 => {
                    counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    reply.extend_from_slice(&1u32.to_be_bytes());
                    reply.extend_from_slice(&tid);
                    reply.extend_from_slice(&1800u32.to_be_bytes());
                    reply.extend_from_slice(&leechers.to_be_bytes());
                    reply.extend_from_slice(&seeders.to_be_bytes());
                    for p in &peers {
                        if let SocketAddr::V4(v4) = p {
                            reply.extend_from_slice(&v4.ip().octets());
                            reply.extend_from_slice(&v4.port().to_be_bytes());
                        }
                    }
                }
                _ => continue,
            }
            let _ = sock.send_to(&reply, from).await;
        }
    });
    UdpStub {
        addr,
        announces,
        _task: task,
    }
}

/// Minimal HTTP tracker (axum): bencoded announce response with compact peers. Records every
/// raw query string it receives.
pub struct HttpStub {
    pub url: String,
    pub queries: Arc<Mutex<Vec<String>>>,
    _task: tokio::task::JoinHandle<()>,
}

type StubState = (Arc<Mutex<Vec<String>>>, Arc<Vec<u8>>);

pub async fn http_stub(
    peers: Vec<SocketAddr>,
    seeders: u32,
    leechers: u32,
    failure: Option<&'static str>,
) -> HttpStub {
    use axum::extract::{RawQuery, State};
    use axum::routing::get;
    let queries = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut body = Vec::new();
    match failure {
        Some(reason) => {
            body.extend_from_slice(
                format!("d14:failure reason{}:{}e", reason.len(), reason).as_bytes(),
            );
        }
        None => {
            let mut compact = Vec::new();
            for p in &peers {
                if let SocketAddr::V4(v4) = p {
                    compact.extend_from_slice(&v4.ip().octets());
                    compact.extend_from_slice(&v4.port().to_be_bytes());
                }
            }
            body.extend_from_slice(
                format!(
                    "d8:completei{seeders}e10:incompletei{leechers}e8:intervali1800e5:peers{}:",
                    compact.len()
                )
                .as_bytes(),
            );
            body.extend_from_slice(&compact);
            body.push(b'e');
        }
    }
    let state = (queries.clone(), Arc::new(body));
    async fn announce(State((queries, body)): State<StubState>, RawQuery(q): RawQuery) -> Vec<u8> {
        queries.lock().push(q.unwrap_or_default());
        (*body).clone()
    }
    let app = axum::Router::new()
        .route("/announce", get(announce))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    HttpStub {
        url: format!("http://{addr}/announce?passkey=SECRET123"),
        queries,
        _task: task,
    }
}
