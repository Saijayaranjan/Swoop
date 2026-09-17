//! `HlsEngine`: downloads permitted, non-DRM HLS media (VOD playlists) segment by segment into a
//! part directory, decrypts AES-128 clear-key segments, checkpoints per segment, then merges.

use crate::detect::{fetch_playlist, variant_label};
use crate::m3u8::{self, KeyMethod, MediaPlaylist, Playlist, Segment};
use crate::merge;
use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use bytes::Bytes;
use osprey_domain::checkpoint::{Checkpoint, HlsCheckpoint};
use osprey_domain::events::LogLevel;
use osprey_domain::media::{MediaInfo, MediaKind};
use osprey_domain::{ErrorKind, Progress, Source, Task, TaskError, TaskKind, TaskState};
use osprey_runtime::backoff::BackoffPolicy;
use osprey_runtime::disk::{FileWriter, FlushLevel, OpenOptionsExt};
use osprey_runtime::engine::{
    CancellationToken, EngineStat, ResolvedMetadata, Transfer, TransferContext, TransferControl,
    TransferOutcome, TransferSecrets,
};
use osprey_runtime::net::{ClientFactory, ClientProfile};
use osprey_runtime::safety;
use osprey_runtime::speed::SpeedMeter;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use url::Url;

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

pub struct HlsEngine;

impl HlsEngine {
    pub fn new() -> Arc<Self> {
        Arc::new(Self)
    }
}

/// The media playlist plus the choices made while resolving it.
struct Resolved {
    media: MediaPlaylist,
    media_url: Url,
    variant_id: String,
    container: &'static str,
    info: MediaInfo,
}

fn playlist_source(task: &Task) -> Result<(Url, Option<String>), TaskError> {
    match &task.source {
        Source::Hls {
            playlist_url,
            variant,
        } => {
            let u = Url::parse(playlist_url)
                .map_err(|e| TaskError::new(ErrorKind::InvalidUrl, e.to_string()))?;
            if !matches!(u.scheme(), "http" | "https") {
                return Err(TaskError::new(
                    ErrorKind::UnsupportedScheme,
                    u.scheme().to_owned(),
                ));
            }
            Ok((u, variant.clone()))
        }
        Source::Urls { urls } => {
            let first = urls
                .first()
                .ok_or_else(|| TaskError::new(ErrorKind::InvalidUrl, "no URL"))?;
            Ok((
                Url::parse(first)
                    .map_err(|e| TaskError::new(ErrorKind::InvalidUrl, e.to_string()))?,
                None,
            ))
        }
        _ => Err(TaskError::new(
            ErrorKind::UnsupportedScheme,
            "not an HLS source",
        )),
    }
}

fn client_for(
    task: &Task,
    clients: &ClientFactory,
    secrets: &TransferSecrets,
) -> Result<reqwest::Client, TaskError> {
    let host = task.source.domain();
    let settings = clients.settings();
    let tls_exception_host = host.filter(|h| {
        settings
            .network
            .tls_exceptions
            .iter()
            .any(|e| e.eq_ignore_ascii_case(h))
    });
    clients.client(&ClientProfile {
        http1_only: false,
        proxy_url: secrets.proxy_url.clone(),
        direct: task.options.direct_connection,
        tls_exception_host,
        cookies: true,
        user_agent: task.options.user_agent.clone(),
    })
}

fn apply_headers(
    mut req: reqwest::RequestBuilder,
    task: &Task,
    secrets: &TransferSecrets,
) -> Result<reqwest::RequestBuilder, TaskError> {
    let headers = osprey_runtime::net::validate_headers(
        task.options
            .headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str())),
    )?;
    for (k, v) in headers {
        req = req.header(k, v);
    }
    for (k, v) in &secrets.headers {
        req = req.header(k, v);
    }
    if let Some(r) = &task.options.referer {
        req = req.header(reqwest::header::REFERER, r);
    }
    if let Some(c) = &task.options.cookies {
        req = req.header(reqwest::header::COOKIE, c);
    }
    if let (Some(u), p) = (&secrets.username, &secrets.password) {
        req = req.basic_auth(u, p.as_deref());
    }
    Ok(req)
}

async fn resolve(
    task: &Task,
    client: &reqwest::Client,
    secrets: &TransferSecrets,
    variant_override: Option<&str>,
) -> Result<Resolved, TaskError> {
    let (url, src_variant) = playlist_source(task)?;
    let wanted = variant_override
        .map(str::to_owned)
        .or_else(|| task.options.media_variant.clone())
        .or(src_variant);
    let _ = secrets; // headers are applied per request below
    let (text, base) = fetch_playlist(client, &url).await?;
    let (media, media_url, variant_id, label) = match m3u8::parse(&text, &base)? {
        Playlist::Media(m) => (m, base.clone(), base.to_string(), "default".to_owned()),
        Playlist::Master(master) => {
            let chosen = wanted
                .as_deref()
                .and_then(|w| {
                    master.variants.iter().find(|v| {
                        v.uri == w || v.name.as_deref() == Some(w) || variant_label(v) == w
                    })
                })
                .or_else(|| {
                    master
                        .variants
                        .iter()
                        .max_by_key(|v| v.average_bandwidth.or(v.bandwidth).unwrap_or(0))
                })
                .ok_or_else(|| TaskError::new(ErrorKind::ParseError, "no variants"))?
                .clone();
            let vu = Url::parse(&chosen.uri)
                .map_err(|e| TaskError::new(ErrorKind::InvalidUrl, e.to_string()))?;
            let (t, b) = fetch_playlist(client, &vu).await?;
            match m3u8::parse(&t, &b)? {
                Playlist::Media(m) => (m, b, chosen.uri.clone(), variant_label(&chosen)),
                Playlist::Master(_) => {
                    return Err(TaskError::new(
                        ErrorKind::ParseError,
                        "nested master playlists are not supported",
                    ))
                }
            }
        }
    };
    if media.protected {
        return Err(TaskError::new(
            ErrorKind::ProtectedContent,
            "playlist uses DRM / SAMPLE-AES; Osprey does not download protected media",
        ));
    }
    if !media.end_list {
        return Err(TaskError::new(
            ErrorKind::LiveStreamUnsupported,
            "playlist has no #EXT-X-ENDLIST (live stream)",
        ));
    }
    let container = if media.init.is_some() { "mp4" } else { "ts" };
    let info = MediaInfo {
        kind: MediaKind::HlsPlaylist,
        title: None,
        format: Some(
            if container == "mp4" {
                "hls-fmp4"
            } else {
                "hls-ts"
            }
            .into(),
        ),
        duration_seconds: Some(media.total_duration),
        variants: Vec::new(),
        selected_variant: Some(label),
        segment_count: Some(media.segments.len() as u32),
        segments_done: 0,
        protected: false,
        page_url: None,
    };
    Ok(Resolved {
        media,
        media_url,
        variant_id,
        container,
        info,
    })
}

fn part_dir_for(task: &Task, temp_suffix: &str) -> PathBuf {
    task.directory.join(format!("{}{}", task.name, temp_suffix))
}

fn segment_path(dir: &Path, i: u32) -> PathBuf {
    dir.join(format!("seg-{i:06}.bin"))
}

fn iv_for(seg: &Segment) -> [u8; 16] {
    if let Some(iv) = seg.key.as_ref().and_then(|k| k.iv) {
        return iv;
    }
    let mut iv = [0u8; 16];
    iv[8..].copy_from_slice(&seg.sequence.to_be_bytes());
    iv
}

struct Shared {
    checkpoint: Mutex<HlsCheckpoint>,
    /// Async mutex held across a key fetch so concurrent workers never fetch the same key twice.
    keys: tokio::sync::Mutex<HashMap<String, [u8; 16]>>,
    bytes_on_disk: std::sync::atomic::AtomicU64,
}

async fn fetch_bytes(
    client: &reqwest::Client,
    task: &Task,
    secrets: &TransferSecrets,
    url: &str,
    range: Option<(u64, u64)>,
    control: &TransferControl,
    limiter: &osprey_runtime::RateLimiter,
) -> Result<Vec<u8>, TaskError> {
    let mut req = apply_headers(client.get(url), task, secrets)?;
    if let Some((off, len)) = range {
        req = req.header(
            reqwest::header::RANGE,
            format!("bytes={}-{}", off, off + len - 1),
        );
    }
    let resp = tokio::select! {
        r = req.send() => r.map_err(|e| classify_reqwest(&e, url))?,
        _ = control.stopped() => return Err(TaskError::cancelled()),
    };
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(TaskError::from_http_status(status, url));
    }
    if range.is_some() && status != 206 {
        return Err(TaskError::new(
            ErrorKind::RangeNotSupported,
            "server ignored byte range for segment",
        )
        .with_source(url.to_owned()));
    }
    let mut out: Vec<u8> = Vec::with_capacity(resp.content_length().unwrap_or(0) as usize);
    let mut stream = resp;
    loop {
        let chunk = tokio::select! {
            c = stream.chunk() => c.map_err(|e| classify_reqwest(&e, url))?,
            _ = control.stopped() => return Err(TaskError::cancelled()),
        };
        let Some(chunk) = chunk else { break };
        limiter.acquire(chunk.len() as u64).await;
        control.counters.add_downloaded(chunk.len() as u64);
        out.extend_from_slice(&chunk);
        if out.len() > 512 * 1024 * 1024 {
            return Err(TaskError::new(
                ErrorKind::ParseError,
                "segment larger than 512 MiB",
            ));
        }
    }
    Ok(out)
}

fn classify_reqwest(e: &reqwest::Error, url: &str) -> TaskError {
    let kind = if e.is_timeout() {
        ErrorKind::ReadTimeout
    } else if e.is_connect() {
        let s = e.to_string().to_ascii_lowercase();
        if s.contains("dns") || s.contains("resolve") {
            ErrorKind::DnsFailure
        } else if s.contains("certificate") {
            ErrorKind::CertificateInvalid
        } else if s.contains("tls") || s.contains("ssl") {
            ErrorKind::TlsFailure
        } else {
            ErrorKind::ConnectionRefused
        }
    } else if e.is_redirect() {
        ErrorKind::RedirectLoop
    } else if e.is_body() || e.is_decode() {
        ErrorKind::Truncated
    } else {
        ErrorKind::ConnectionReset
    };
    TaskError::new(kind, osprey_runtime::redact::redact(&e.to_string()))
        .with_source(osprey_runtime::redact::redact(url))
}

async fn key_for(
    seg: &Segment,
    shared: &Shared,
    client: &reqwest::Client,
    task: &Task,
    secrets: &TransferSecrets,
    control: &TransferControl,
    limiter: &osprey_runtime::RateLimiter,
) -> Result<Option<[u8; 16]>, TaskError> {
    let Some(key) = &seg.key else { return Ok(None) };
    if key.method != KeyMethod::Aes128 {
        return Err(TaskError::new(
            ErrorKind::ProtectedContent,
            "unsupported key method",
        ));
    }
    let uri = key
        .uri
        .clone()
        .ok_or_else(|| TaskError::new(ErrorKind::ParseError, "AES-128 key without URI"))?;
    let mut keys = shared.keys.lock().await;
    if let Some(k) = keys.get(&uri) {
        return Ok(Some(*k));
    }
    // cached in the checkpoint (key URLs expire)
    let cached = shared.checkpoint.lock().key_cache.get(&uri).cloned();
    if let Some(hex) = cached {
        if let Ok(bytes) = hex::decode(&hex) {
            if bytes.len() == 16 {
                let mut k = [0u8; 16];
                k.copy_from_slice(&bytes);
                keys.insert(uri, k);
                return Ok(Some(k));
            }
        }
    }
    let bytes = fetch_bytes(client, task, secrets, &uri, None, control, limiter).await?;
    if bytes.len() != 16 {
        return Err(TaskError::new(
            ErrorKind::ParseError,
            format!("AES-128 key has {} bytes", bytes.len()),
        ));
    }
    let mut k = [0u8; 16];
    k.copy_from_slice(&bytes);
    keys.insert(uri.clone(), k);
    shared
        .checkpoint
        .lock()
        .key_cache
        .insert(uri, hex::encode(k));
    Ok(Some(k))
}

fn decrypt(data: &[u8], key: &[u8; 16], iv: &[u8; 16]) -> Result<Vec<u8>, TaskError> {
    Aes128CbcDec::new(key.into(), iv.into())
        .decrypt_padded_vec_mut::<Pkcs7>(data)
        .map_err(|e| TaskError::new(ErrorKind::ParseError, format!("AES-128 decrypt: {e}")))
}

async fn write_segment(path: &Path, root: &Path, data: Vec<u8>) -> Result<u64, TaskError> {
    let w = FileWriter::open(
        path,
        OpenOptionsExt {
            preallocate: None,
            sparse: false,
            in_flight_bytes: None,
            expected_root: Some(root.to_path_buf()),
        },
    )
    .await?;
    let len = data.len() as u64;
    w.truncate(0).await?;
    w.write(0, Bytes::from(data)).await?;
    w.flush(FlushLevel::Barrier).await?;
    w.close().await?;
    Ok(len)
}

#[async_trait::async_trait]
impl Transfer for HlsEngine {
    fn kinds(&self) -> &'static [TaskKind] {
        &[TaskKind::Hls]
    }

    async fn probe(
        &self,
        task: &Task,
        settings: Arc<osprey_domain::settings::Settings>,
        secrets: TransferSecrets,
        clients: Arc<ClientFactory>,
    ) -> Result<ResolvedMetadata, TaskError> {
        let _ = settings;
        let client = client_for(task, &clients, &secrets)?;
        let r = resolve(task, &client, &secrets, None).await?;
        let mut info = r.info.clone();
        // variants for the picker
        if let Ok(desc) =
            crate::detect::describe_hls(playlist_source(task)?.0.as_str(), clients.clone()).await
        {
            info.variants = desc.variants;
        }
        let name = suggested_name(task, r.container);
        Ok(ResolvedMetadata {
            name: Some(name),
            total: None,
            mime: Some(
                if r.container == "mp4" {
                    "video/mp4"
                } else {
                    "video/mp2t"
                }
                .into(),
            ),
            resumable: Some(true),
            final_url: Some(r.media_url.to_string()),
            media: Some(info),
            ..Default::default()
        })
    }

    async fn run(&self, ctx: TransferContext) -> TransferOutcome {
        match self.run_inner(&ctx).await {
            Ok(o) => o,
            Err(e) if e.kind == ErrorKind::Cancelled => ctx.control.stop_outcome(),
            Err(e) => TransferOutcome::Failed(e),
        }
    }
}

fn suggested_name(task: &Task, container: &str) -> String {
    let base = if task.name.is_empty() {
        playlist_source(task)
            .ok()
            .and_then(|(u, _)| osprey_runtime::filename::from_url(u.as_str()))
            .unwrap_or_else(|| "video".into())
    } else {
        task.name.clone()
    };
    let stem = Path::new(&base)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video")
        .to_owned();
    let stem = if stem.eq_ignore_ascii_case("index")
        || stem.eq_ignore_ascii_case("playlist")
        || stem.eq_ignore_ascii_case("master")
    {
        task.source
            .domain()
            .map(|d| d.replace('.', "-"))
            .unwrap_or_else(|| "video".into())
    } else {
        stem
    };
    safety::sanitize_filename(&format!("{stem}.{container}"))
}

/// The output name: the user's name if locked or already sensible, otherwise derived from the
/// playlist URL; the extension always matches the container we will actually produce.
fn effective_name(task: &Task, container: &str) -> String {
    if task.name.is_empty() || !task.name.contains('.') {
        return suggested_name(task, container);
    }
    if task.name_locked {
        return task.name.clone();
    }
    let p = Path::new(&task.name);
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let matches_container = match container {
        "mp4" => matches!(ext.as_str(), "mp4" | "m4v" | "mov" | "m4a"),
        _ => matches!(ext.as_str(), "ts" | "mpg" | "mpeg"),
    };
    if matches_container {
        return task.name.clone();
    }
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("video");
    safety::sanitize_filename(&format!("{stem}.{container}"))
}

impl HlsEngine {
    async fn run_inner(&self, ctx: &TransferContext) -> Result<TransferOutcome, TaskError> {
        let task = &ctx.task;
        let control = ctx.control.clone();
        let sink = ctx.sink.clone();
        let settings = ctx.settings.clone();
        safety::validate_destination_dir(&task.directory)?;
        sink.state(TaskState::Resolving, None);
        let client = client_for(task, &ctx.clients, &ctx.secrets)?;
        let resolved = resolve(task, &client, &ctx.secrets, None).await?;
        let media = resolved.media;
        let total_segments = media.segments.len() as u32;
        let name = effective_name(task, resolved.container);
        let part_dir = part_dir_for(task, &settings.storage.temp_suffix);
        safety::ensure_within(&task.directory, &part_dir)?;
        tokio::fs::create_dir_all(&part_dir)
            .await
            .map_err(|e| TaskError::from_io(&e, "create part dir"))?;

        // resume?
        let mut cp = match &ctx.checkpoint {
            Some(Checkpoint::Hls(h))
                if h.media_playlist_url == resolved.media_url.as_str()
                    && h.segment_count == total_segments =>
            {
                h.clone()
            }
            Some(Checkpoint::Hls(_)) => {
                sink.log(
                    LogLevel::Warn,
                    "hls.checkpoint_mismatch",
                    "playlist changed; starting over".into(),
                );
                HlsCheckpoint::default()
            }
            _ => HlsCheckpoint::default(),
        };
        cp.media_playlist_url = resolved.media_url.to_string();
        cp.part_dir = part_dir.clone();
        cp.segment_count = total_segments;
        cp.container = resolved.container.to_owned();
        // validate on-disk segments claimed done
        let mut bytes_on_disk = 0u64;
        for i in 0..total_segments {
            if cp.is_done(i) {
                match tokio::fs::metadata(segment_path(&part_dir, i)).await {
                    Ok(m) if m.len() > 0 => bytes_on_disk += m.len(),
                    _ => {
                        // missing file: clear the bit
                        let idx = (i / 8) as usize;
                        if let Some(b) = cp.done_bitmap.get_mut(idx) {
                            *b &= !(1 << (i % 8));
                        }
                        cp.segment_sizes.remove(&i);
                    }
                }
            }
        }
        let mut info = resolved.info.clone();
        info.segments_done = cp.done_count();
        sink.metadata(ResolvedMetadata {
            name: Some(name.clone()),
            total: None,
            mime: Some(
                if resolved.container == "mp4" {
                    "video/mp4"
                } else {
                    "video/mp2t"
                }
                .into(),
            ),
            resumable: Some(true),
            final_url: Some(resolved.media_url.to_string()),
            media: Some(info),
            ..Default::default()
        });
        sink.log(
            LogLevel::Info,
            "hls.resolved",
            format!(
                "{} segments, variant {}",
                total_segments,
                osprey_runtime::redact::redact(&resolved.variant_id)
            ),
        );

        let shared = Arc::new(Shared {
            checkpoint: Mutex::new(cp),
            keys: tokio::sync::Mutex::new(HashMap::new()),
            bytes_on_disk: std::sync::atomic::AtomicU64::new(bytes_on_disk),
        });
        control.counters.set_downloaded(bytes_on_disk);

        // init segment
        if let Some(init) = &media.init {
            if !shared.checkpoint.lock().init_segment_done {
                sink.state(TaskState::Connecting, None);
                let range = init.byte_range.as_ref().map(|r| (r.offset, r.length));
                let data = fetch_bytes(
                    &client,
                    task,
                    &ctx.secrets,
                    &init.uri,
                    range,
                    &control,
                    &ctx.limiter,
                )
                .await?;
                let n = write_segment(&part_dir.join("init.bin"), &task.directory, data).await?;
                shared.bytes_on_disk.fetch_add(n, Ordering::Relaxed);
                let mut c = shared.checkpoint.lock();
                c.init_segment_done = true;
                sink.checkpoint(Checkpoint::Hls(c.clone()));
            }
        }

        sink.state(TaskState::Downloading, None);
        let concurrency = {
            let c = control.max_connections();
            let c = if c == 0 {
                settings.network.connections_per_task
            } else {
                c
            };
            (c as usize).clamp(1, 8)
        };
        let sem = Arc::new(Semaphore::new(concurrency));
        let backoff = BackoffPolicy::from_settings(&settings.network);
        let pending: Vec<Segment> = media
            .segments
            .iter()
            .filter(|s| !shared.checkpoint.lock().is_done(s.index))
            .cloned()
            .collect();
        let mut handles = Vec::new();
        let speed = Arc::new(Mutex::new(SpeedMeter::new()));
        // Internal stop signal: when one segment fails permanently the other workers stop, but
        // the task outcome is decided from `first_error`, not from the user's control token.
        let abort = CancellationToken::new();

        // progress reporter
        let reporter = {
            let shared = shared.clone();
            let sink = sink.clone();
            let control = control.clone();
            let speed = speed.clone();
            let total_duration = media.total_duration;
            tokio::spawn(async move {
                let mut last = control.counters.downloaded();
                loop {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    let now = control.counters.downloaded();
                    let mut sp = speed.lock();
                    sp.record(now.saturating_sub(last));
                    last = now;
                    let s = sp.tick();
                    let done = shared.checkpoint.lock().done_count();
                    let on_disk = shared.bytes_on_disk.load(Ordering::Relaxed);
                    let fraction = if total_segments > 0 {
                        done as f32 / total_segments as f32
                    } else {
                        0.0
                    };
                    // size estimate from bytes so far vs duration covered
                    let estimated_total = if done > 0 {
                        Some(((on_disk as f64) / (done as f64) * total_segments as f64) as u64)
                    } else {
                        None
                    };
                    let _ = total_duration;
                    let remaining = estimated_total
                        .map(|t| t.saturating_sub(on_disk))
                        .unwrap_or(0);
                    sink.progress(Progress {
                        downloaded: on_disk,
                        total: estimated_total,
                        speed: s,
                        instant_speed: sp.instant(),
                        eta_seconds: if s > 0 && remaining > 0 {
                            Some(remaining / s)
                        } else {
                            None
                        },
                        active_connections: control
                            .counters
                            .active_connections
                            .load(Ordering::Relaxed)
                            as u32,
                        fraction,
                        ..Default::default()
                    });
                    sink.stat(EngineStat::Peak { speed: sp.peak() });
                }
            })
        };

        for seg in pending {
            let permit = tokio::select! {
                p = sem.clone().acquire_owned() => p.map_err(|_| TaskError::internal("semaphore closed"))?,
                _ = control.stopped() => break,
                _ = abort.cancelled() => break,
            };
            let abort = abort.clone();
            let client = client.clone();
            let task = task.clone();
            let secrets = ctx.secrets.clone();
            let control = control.clone();
            let limiter = ctx.limiter.clone();
            let shared = shared.clone();
            let sink = sink.clone();
            let part_dir = part_dir.clone();
            let root = task.directory.clone();
            let backoff = backoff.clone();
            handles.push(tokio::spawn(async move {
                let _permit = permit;
                let mut attempt = 0u32;
                loop {
                    if control.should_stop() || abort.is_cancelled() {
                        return Err(TaskError::cancelled());
                    }
                    control
                        .counters
                        .active_connections
                        .fetch_add(1, Ordering::Relaxed);
                    let work = async {
                        let range = seg.byte_range.as_ref().map(|r| (r.offset, r.length));
                        let raw = fetch_bytes(
                            &client, &task, &secrets, &seg.uri, range, &control, &limiter,
                        )
                        .await?;
                        let data = match key_for(
                            &seg, &shared, &client, &task, &secrets, &control, &limiter,
                        )
                        .await?
                        {
                            Some(k) => decrypt(&raw, &k, &iv_for(&seg))?,
                            None => raw,
                        };
                        write_segment(&segment_path(&part_dir, seg.index), &root, data).await
                    };
                    let result = tokio::select! {
                        r = work => r,
                        _ = abort.cancelled() => Err(TaskError::cancelled()),
                    };
                    control
                        .counters
                        .active_connections
                        .fetch_sub(1, Ordering::Relaxed);
                    match result {
                        Ok(n) => {
                            shared.bytes_on_disk.fetch_add(n, Ordering::Relaxed);
                            let mut c = shared.checkpoint.lock();
                            c.set_done(seg.index);
                            c.segment_sizes.insert(seg.index, n);
                            sink.checkpoint(Checkpoint::Hls(c.clone()));
                            sink.stat(EngineStat::ConnectionOpened);
                            return Ok(());
                        }
                        Err(e) if e.kind == ErrorKind::Cancelled => return Err(e),
                        Err(e) => {
                            attempt += 1;
                            sink.stat(EngineStat::ConnectionFailed);
                            let delay = backoff.delay_for_with_hint(
                                attempt,
                                e.class(),
                                e.retry_after_ms.map(Duration::from_millis),
                            );
                            match delay {
                                Some(d) if e.is_retryable() => {
                                    sink.log(
                                        LogLevel::Warn,
                                        "hls.segment_retry",
                                        format!(
                                            "segment {} attempt {attempt}: {}",
                                            seg.index, e.message
                                        ),
                                    );
                                    sink.stat(EngineStat::Retry);
                                    tokio::select! {
                                        _ = tokio::time::sleep(d) => {},
                                        _ = control.stopped() => return Err(TaskError::cancelled()),
                                    }
                                }
                                _ => return Err(e.with_detail(format!("segment {}", seg.index))),
                            }
                        }
                    }
                }
            }));
        }

        let mut first_error: Option<TaskError> = None;
        for h in handles {
            match h.await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    if e.kind != ErrorKind::Cancelled && first_error.is_none() {
                        first_error = Some(e);
                    }
                    abort.cancel();
                }
                Err(e) => {
                    if first_error.is_none() {
                        first_error =
                            Some(TaskError::internal(format!("segment task panicked: {e}")));
                    }
                }
            }
        }
        reporter.abort();
        // final checkpoint
        sink.checkpoint(Checkpoint::Hls(shared.checkpoint.lock().clone()));
        if control.should_stop() {
            return Ok(control.stop_outcome());
        }
        if let Some(e) = first_error {
            return Err(e);
        }
        if shared.checkpoint.lock().done_count() != total_segments {
            return Err(TaskError::new(
                ErrorKind::Truncated,
                "not all segments were downloaded",
            ));
        }

        // merge
        sink.state(TaskState::Processing, Some("Merging segments".into()));
        let mut parts = Vec::with_capacity(total_segments as usize + 1);
        if media.init.is_some() {
            parts.push(part_dir.join("init.bin"));
        }
        for i in 0..total_segments {
            parts.push(segment_path(&part_dir, i));
        }
        let merged_tmp = task.directory.join(format!(".{}.osprey-merge", name));
        let bytes = merge::concatenate(&parts, &merged_tmp).await?;
        let mut final_name = name.clone();
        let mut final_src = merged_tmp.clone();
        if resolved.container == "ts" {
            if let Some(ffmpeg) = merge::find_ffmpeg() {
                let mp4_tmp = task.directory.join(format!(".{}.osprey-remux.mp4", name));
                sink.state(TaskState::Processing, Some("Remuxing to MP4".into()));
                match merge::remux_to_mp4(&ffmpeg, &merged_tmp, &mp4_tmp).await {
                    Ok(true) => {
                        let _ = tokio::fs::remove_file(&merged_tmp).await;
                        final_src = mp4_tmp;
                        final_name = format!(
                            "{}.mp4",
                            Path::new(&name)
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("video")
                        );
                        sink.log(
                            LogLevel::Info,
                            "hls.remuxed",
                            "remuxed TS to MP4 with ffmpeg".into(),
                        );
                    }
                    Ok(false) => sink.log(
                        LogLevel::Warn,
                        "hls.remux_failed",
                        "ffmpeg could not remux; keeping .ts".into(),
                    ),
                    Err(e) => sink.log(LogLevel::Warn, "hls.remux_failed", e.message),
                }
            }
        }
        let final_size = tokio::fs::metadata(&final_src)
            .await
            .map(|m| m.len())
            .unwrap_or(bytes);
        if settings.storage.quarantine_downloads {
            if let Err(e) = osprey_runtime::disk::set_quarantine(&final_src, "Osprey", None) {
                sink.log(LogLevel::Warn, "quarantine.failed", e.to_string());
            }
        }
        let mut final_path = task.directory.join(&final_name);
        safety::ensure_within(&task.directory, &final_path)?;
        if tokio::fs::metadata(&final_path).await.is_ok() {
            final_path = safety::unique_path(&final_path);
        }
        tokio::fs::rename(&final_src, &final_path)
            .await
            .map_err(|e| TaskError::from_io(&e, "rename merged file"))?;
        let _ = tokio::fs::remove_dir_all(&part_dir).await;
        sink.log(
            LogLevel::Info,
            "hls.completed",
            format!("{} bytes", final_size),
        );
        Ok(TransferOutcome::Completed {
            file_path: final_path,
            bytes: final_size,
        })
    }
}
