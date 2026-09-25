//! `FtpEngine`: single-connection FTP/FTPS transfer with `REST` resume, the pause protocol and
//! hierarchical rate limiting.

use crate::client::{FtpClient, FtpEntry, Security};
use bytes::Bytes;
use percent_encoding::percent_decode_str;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use swoop_domain::checkpoint::Checkpoint;
use swoop_domain::events::LogLevel;
use swoop_domain::settings::Settings;
use swoop_domain::{
    ErrorKind, IoContext, Progress, Segment, SegmentMap, Source, Task, TaskError, TaskKind,
    TaskState,
};
use swoop_runtime::backoff::BackoffPolicy;
use swoop_runtime::disk::{FileWriter, FlushLevel, OpenOptionsExt};
use swoop_runtime::engine::{
    EngineStat, ResolvedMetadata, Transfer, TransferContext, TransferOutcome, TransferSecrets,
};
use swoop_runtime::net::ClientFactory;
use swoop_runtime::safety;
use swoop_runtime::speed::SpeedMeter;
use tokio::io::AsyncReadExt;
use url::Url;

pub struct FtpEngine;

impl FtpEngine {
    pub fn new() -> Arc<Self> {
        Arc::new(Self)
    }

    /// Directory listing for the add dialog.
    pub async fn list(
        url: &str,
        settings: &Settings,
        secrets: &TransferSecrets,
    ) -> Result<Vec<FtpEntry>, TaskError> {
        let target = Target::parse(url, secrets)?;
        let mut client = target.connect(settings).await?;
        let r = client.list(&target.path).await;
        client.quit().await;
        r
    }
}

struct Target {
    host: String,
    port: u16,
    security: Security,
    path: String,
    user: String,
    pass: String,
}

impl Target {
    fn parse(url: &str, secrets: &TransferSecrets) -> Result<Self, TaskError> {
        let u =
            Url::parse(url).map_err(|e| TaskError::new(ErrorKind::InvalidUrl, e.to_string()))?;
        let security = match u.scheme() {
            "ftp" => Security::None,
            "ftps" => {
                if u.port() == Some(990) {
                    Security::Implicit
                } else {
                    Security::Explicit
                }
            }
            "ftpes" => Security::Explicit,
            other => {
                return Err(TaskError::new(
                    ErrorKind::UnsupportedScheme,
                    other.to_owned(),
                ))
            }
        };
        let host = u
            .host_str()
            .ok_or_else(|| TaskError::new(ErrorKind::InvalidUrl, "missing host"))?
            .to_owned();
        let port = u.port().unwrap_or(if security == Security::Implicit {
            990
        } else {
            21
        });
        let (user, pass) = if !u.username().is_empty() {
            (
                percent_decode_str(u.username())
                    .decode_utf8_lossy()
                    .into_owned(),
                u.password()
                    .map(|p| percent_decode_str(p).decode_utf8_lossy().into_owned())
                    .unwrap_or_default(),
            )
        } else if let Some(user) = &secrets.username {
            (user.clone(), secrets.password.clone().unwrap_or_default())
        } else {
            ("anonymous".to_owned(), "swoop@example.com".to_owned())
        };
        let path = percent_decode_str(u.path())
            .decode_utf8_lossy()
            .into_owned();
        if path.contains('\r') || path.contains('\n') {
            return Err(TaskError::new(
                ErrorKind::InvalidUrl,
                "control characters in path",
            ));
        }
        Ok(Self {
            host,
            port,
            security,
            path,
            user,
            pass,
        })
    }

    async fn connect(&self, settings: &Settings) -> Result<FtpClient, TaskError> {
        let verify = settings.network.verify_tls
            && !settings
                .network
                .tls_exceptions
                .iter()
                .any(|h| h.eq_ignore_ascii_case(&self.host));
        FtpClient::connect(
            &self.host,
            self.port,
            self.security,
            verify,
            &self.user,
            &self.pass,
            Duration::from_secs(settings.network.connect_timeout_seconds.max(5) as u64),
        )
        .await
    }
}

fn source_url(task: &Task) -> Result<String, TaskError> {
    match &task.source {
        Source::Urls { urls } => urls
            .first()
            .cloned()
            .ok_or_else(|| TaskError::new(ErrorKind::InvalidUrl, "no URL")),
        _ => Err(TaskError::new(
            ErrorKind::UnsupportedScheme,
            "not an FTP source",
        )),
    }
}

#[async_trait::async_trait]
impl Transfer for FtpEngine {
    fn kinds(&self) -> &'static [TaskKind] {
        &[TaskKind::Ftp]
    }

    async fn probe(
        &self,
        task: &Task,
        settings: Arc<Settings>,
        secrets: TransferSecrets,
        _clients: Arc<ClientFactory>,
    ) -> Result<ResolvedMetadata, TaskError> {
        let url = source_url(task)?;
        let target = Target::parse(&url, &secrets)?;
        let mut client = target.connect(&settings).await?;
        let size = client.size(&target.path).await?;
        let modified = client.mdtm(&target.path).await.ok().flatten();
        let resumable = client.supports("REST");
        client.quit().await;
        Ok(ResolvedMetadata {
            name: swoop_runtime::filename::from_url(&url),
            total: size,
            mime: swoop_runtime::filename::from_url(&url)
                .and_then(|n| swoop_runtime::filename::mime_for_name(&n)),
            resumable: Some(resumable),
            final_url: Some(swoop_runtime::redact::redact(&url)),
            last_modified: modified,
            remote_addr: Some(format!("{}:{}", target.host, target.port)),
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

impl FtpEngine {
    async fn run_inner(&self, ctx: &TransferContext) -> Result<TransferOutcome, TaskError> {
        let task = &ctx.task;
        let control = ctx.control.clone();
        let sink = ctx.sink.clone();
        let settings = ctx.settings.clone();
        safety::validate_destination_dir(&task.directory)?;
        let url = source_url(task)?;
        let target = Target::parse(&url, &ctx.secrets)?;
        let name = if task.name.is_empty() {
            swoop_runtime::filename::from_url(&url).unwrap_or_else(|| "download".into())
        } else {
            task.name.clone()
        };
        let part_path: PathBuf = task
            .directory
            .join(format!("{}{}", name, settings.storage.temp_suffix));
        safety::ensure_within(&task.directory, &part_path)?;

        sink.state(TaskState::Connecting, None);
        let mut client = target.connect(&settings).await?;
        let total = client.size(&target.path).await?;
        let modified = client.mdtm(&target.path).await.ok().flatten();
        let rest_supported = client.supports("REST");
        sink.stat(EngineStat::RangeSupport {
            supported: rest_supported,
        });
        sink.metadata(ResolvedMetadata {
            name: Some(name.clone()),
            total,
            resumable: Some(rest_supported),
            last_modified: modified.clone(),
            remote_addr: Some(format!("{}:{}", target.host, target.port)),
            ..Default::default()
        });

        // resume decision
        let mut committed = 0u64;
        if let Some(Checkpoint::Segments(map)) = &ctx.checkpoint {
            let same = map.total == total
                && (map.last_modified.is_none() || map.last_modified == modified);
            let part_len = tokio::fs::metadata(&part_path)
                .await
                .map(|m| m.len())
                .unwrap_or(0);
            if same && rest_supported {
                committed = map.committed_bytes().min(part_len);
            } else if !same {
                sink.log(
                    LogLevel::Warn,
                    "ftp.source_changed",
                    "size or mtime changed; restarting".into(),
                );
            }
        }
        if let Some(t) = total {
            if committed >= t && t > 0 {
                committed = 0; // do not trust a claim of completeness without verification; refetch
            }
        }
        let writer = FileWriter::open(
            &part_path,
            OpenOptionsExt {
                preallocate: if settings.storage.preallocate {
                    total
                } else {
                    None
                },
                sparse: settings.storage.sparse_files,
                in_flight_bytes: None,
                expected_root: Some(task.directory.clone()),
            },
        )
        .await?;
        if committed == 0 {
            writer.truncate(0).await?;
        }
        let mut map = SegmentMap {
            segments: vec![Segment::new(0, 0, total.unwrap_or(u64::MAX))],
            etag: None,
            last_modified: modified.clone(),
            total,
            part_path: Some(part_path.clone()),
        };
        map.segments[0].committed = committed;
        sink.checkpoint(Checkpoint::Segments(map.clone()));

        let backoff = BackoffPolicy::from_settings(&settings.network);
        let mut attempt = 0u32;
        let mut written = committed; // bytes written to the writer (not necessarily flushed)
        let mut speed = SpeedMeter::new();
        let mut last_tick = std::time::Instant::now();
        let mut last_checkpoint = std::time::Instant::now();
        sink.state(TaskState::Downloading, None);
        control.counters.set_downloaded(written);
        let chunk_size = ctx.limiter.suggested_chunk();
        let mut buf = vec![0u8; chunk_size.max(4096)];

        'outer: loop {
            if control.should_stop() {
                break;
            }
            let mut data = match client.retr(&target.path, written).await {
                Ok(d) => d,
                Err(e) if e.kind == ErrorKind::RangeNotSupported && written > 0 => {
                    sink.log(
                        LogLevel::Warn,
                        "ftp.no_resume",
                        "server refused REST; restarting from 0".into(),
                    );
                    sink.stat(EngineStat::RangeSupport { supported: false });
                    written = 0;
                    map.segments[0].committed = 0;
                    writer.truncate(0).await?;
                    control.counters.set_downloaded(0);
                    continue;
                }
                Err(e) => {
                    attempt += 1;
                    match backoff.delay_for(attempt, e.class()) {
                        Some(d) if e.is_retryable() => {
                            sink.stat(EngineStat::Retry);
                            sink.log(
                                LogLevel::Warn,
                                "ftp.retry",
                                format!("attempt {attempt}: {}", e.message),
                            );
                            tokio::select! { _ = tokio::time::sleep(d) => {}, _ = control.stopped() => break 'outer }
                            client = match target.connect(&settings).await {
                                Ok(c) => c,
                                Err(e2) => {
                                    if backoff.delay_for(attempt + 1, e2.class()).is_none() {
                                        return Err(e2);
                                    }
                                    continue;
                                }
                            };
                            continue;
                        }
                        _ => return Err(e),
                    }
                }
            };
            sink.stat(EngineStat::ConnectionOpened);
            control
                .counters
                .active_connections
                .store(1, std::sync::atomic::Ordering::Relaxed);
            let read_timeout =
                Duration::from_secs(settings.network.read_timeout_seconds.max(5) as u64);
            loop {
                let n = tokio::select! {
                    r = tokio::time::timeout(read_timeout, data.read(&mut buf)) => match r {
                        Ok(Ok(n)) => n,
                        Ok(Err(e)) => {
                            sink.stat(EngineStat::ConnectionFailed);
                            let err = TaskError::from_io_ctx(&e, "data read", IoContext::Network);
                            attempt += 1;
                            match backoff.delay_for(attempt, err.class()) {
                                Some(d) => {
                                    sink.log(LogLevel::Warn, "ftp.reconnect", format!("data connection lost at {written}: {}", err.message));
                                    drop(data);
                                    writer.flush(FlushLevel::Barrier).await?;
                                    map.segments[0].committed = written;
                                    sink.checkpoint(Checkpoint::Segments(map.clone()));
                                    tokio::select! { _ = tokio::time::sleep(d) => {}, _ = control.stopped() => break 'outer }
                                    client = target.connect(&settings).await?;
                                    continue 'outer;
                                }
                                None => return Err(err),
                            }
                        }
                        Err(_) => {
                            attempt += 1;
                            sink.stat(EngineStat::ConnectionFailed);
                            if backoff.delay_for(attempt, swoop_domain::FailureClass::Transient).is_none() {
                                return Err(TaskError::new(ErrorKind::ReadTimeout, "data read timed out"));
                            }
                            drop(data);
                            writer.flush(FlushLevel::Barrier).await?;
                            map.segments[0].committed = written;
                            sink.checkpoint(Checkpoint::Segments(map.clone()));
                            client = target.connect(&settings).await?;
                            continue 'outer;
                        }
                    },
                    _ = control.stopped() => {
                        drop(data);
                        client.abort().await;
                        break 'outer;
                    }
                };
                if n == 0 {
                    break;
                }
                ctx.limiter.acquire(n as u64).await;
                writer
                    .write(written, Bytes::copy_from_slice(&buf[..n]))
                    .await?;
                written += n as u64;
                control.counters.add_downloaded(n as u64);
                speed.record(n as u64);
                if last_tick.elapsed() >= Duration::from_millis(500) {
                    last_tick = std::time::Instant::now();
                    let s = speed.tick();
                    sink.progress(Progress {
                        downloaded: written,
                        total,
                        speed: s,
                        instant_speed: speed.instant(),
                        eta_seconds: total
                            .map(|t| t.saturating_sub(written))
                            .and_then(|r| speed.eta(r)),
                        active_connections: 1,
                        ..Default::default()
                    });
                    sink.stat(EngineStat::Peak {
                        speed: speed.peak(),
                    });
                }
                if last_checkpoint.elapsed() >= Duration::from_secs(3) {
                    last_checkpoint = std::time::Instant::now();
                    writer.flush(FlushLevel::Barrier).await?;
                    map.segments[0].committed = written;
                    sink.checkpoint(Checkpoint::Segments(map.clone()));
                }
                if let Some(t) = total {
                    if written >= t {
                        break;
                    }
                }
            }
            drop(data);
            let finish = client.finish_transfer().await;
            let short = total.map(|t| written < t).unwrap_or(false);
            if finish.is_err() || short {
                // The server closed the data connection early (EOF looks clean on our side).
                attempt += 1;
                let err = finish.err().unwrap_or_else(|| {
                    TaskError::new(
                        ErrorKind::ConnectionReset,
                        format!("data connection closed at {written}"),
                    )
                });
                sink.stat(EngineStat::ConnectionFailed);
                match backoff.delay_for(attempt, swoop_domain::FailureClass::Transient) {
                    Some(d) => {
                        sink.log(
                            LogLevel::Warn,
                            "ftp.reconnect",
                            format!("transfer ended early at {written}: {}", err.message),
                        );
                        writer.flush(FlushLevel::Barrier).await?;
                        map.segments[0].committed = written;
                        sink.checkpoint(Checkpoint::Segments(map.clone()));
                        tokio::select! { _ = tokio::time::sleep(d) => {}, _ = control.stopped() => break 'outer }
                        client = target.connect(&settings).await?;
                        continue 'outer;
                    }
                    None => return Err(err),
                }
            }
            break;
        }

        // pause / cancel path
        if control.should_stop() {
            match tokio::time::timeout(Duration::from_secs(5), writer.flush(FlushLevel::Barrier))
                .await
            {
                Ok(Ok(())) => {
                    map.segments[0].committed = written;
                    sink.checkpoint(Checkpoint::Segments(map.clone()));
                }
                _ => sink.log(
                    LogLevel::Warn,
                    "ftp.pause_flush_timeout",
                    "flush did not complete; keeping previous checkpoint".into(),
                ),
            }
            let _ = writer.close().await;
            client.quit().await;
            return Ok(control.stop_outcome());
        }
        if let Some(t) = total {
            if written != t {
                return Err(TaskError::new(
                    ErrorKind::Truncated,
                    format!("received {written} of {t} bytes"),
                ));
            }
        }
        sink.state(TaskState::Verifying, None);
        writer.flush(FlushLevel::Full).await?;
        writer.close().await?;
        map.segments[0].end = written;
        map.segments[0].committed = written;
        sink.checkpoint(Checkpoint::Segments(map));
        client.quit().await;
        if settings.storage.quarantine_downloads {
            if let Err(e) = swoop_runtime::disk::set_quarantine(&part_path, "Swoop", Some(&url)) {
                sink.log(LogLevel::Warn, "quarantine.failed", e.to_string());
            }
        }
        let mut final_path = task.directory.join(&name);
        safety::ensure_within(&task.directory, &final_path)?;
        if tokio::fs::metadata(&final_path).await.is_ok() {
            final_path = safety::unique_path(&final_path);
        }
        tokio::fs::rename(&part_path, &final_path)
            .await
            .map_err(|e| TaskError::from_io(&e, "rename"))?;
        Ok(TransferOutcome::Completed {
            file_path: final_path,
            bytes: written,
        })
    }
}
