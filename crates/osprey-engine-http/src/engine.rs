//! `HttpEngine`: the [`Transfer`] implementation for `TaskKind::Http` and `TaskKind::Metalink`.
//!
//! A run is: resolve sources (URLs or a metalink) → probe (with retry) → decide the plan (fresh
//! or resumed from the checkpoint) → open the part file → hand everything to the
//! [`Controller`] → finalise (full flush, optional hash verification, quarantine, rename).
//! Anything that fails is turned into a classified [`TaskError`] carrying the source URL.

use crate::controller::{Controller, RunResult};
use crate::hostlimits::HostLimits;
use crate::metalink::{self, MetalinkFile};
use crate::mirrors::{self, MirrorSet};
use crate::plan::{initial_segment_count, Plan};
use crate::probe::{self, data_url, ProbeResult};
use crate::request::{client_profile, parse_http_url, RequestTemplate};
use crate::shared::{RunShared, Tuning};
use osprey_domain::checkpoint::Checkpoint;
use osprey_domain::events::LogLevel;
use osprey_domain::settings::Settings;
use osprey_domain::{
    ErrorKind, FailureClass, IoContext, SegmentMap, Source, Task, TaskError, TaskKind, TaskState,
};
use osprey_runtime::backoff::BackoffPolicy;
use osprey_runtime::disk::{set_quarantine, FileWriter, FlushLevel, OpenOptionsExt};
use osprey_runtime::engine::{
    EngineStat, ProgressSink, ResolvedMetadata, Transfer, TransferContext, TransferControl,
    TransferOutcome, TransferSecrets,
};
use osprey_runtime::net::ClientFactory;
use osprey_runtime::redact::redact;
use osprey_runtime::safety::{ensure_within, sanitize_filename, unique_path};
use parking_lot::Mutex;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;
use std::time::Duration;

/// Timeout for the add-dialog probe (not a transfer; the user is waiting).
const DIALOG_PROBE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone)]
pub struct HttpEngine {
    tuning: Tuning,
    limits: Arc<HostLimits>,
}

impl Default for HttpEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpEngine {
    /// Production engine: default timings, process-wide connection caps.
    pub fn new() -> Self {
        Self {
            tuning: Tuning::default(),
            limits: HostLimits::global(),
        }
    }

    pub fn with_tuning(mut self, tuning: Tuning) -> Self {
        self.tuning = tuning;
        self
    }

    /// Use a private connection-cap registry (tests, embedded use).
    pub fn with_limits(mut self, limits: Arc<HostLimits>) -> Self {
        self.limits = limits;
        self
    }

    pub fn tuning(&self) -> &Tuning {
        &self.tuning
    }

    /// Probe a bare URL (add dialog, media detection) with the given settings and secrets.
    pub async fn probe_url(
        url: &str,
        settings: Arc<Settings>,
        clients: Arc<ClientFactory>,
        secrets: &TransferSecrets,
    ) -> Result<ResolvedMetadata, TaskError> {
        let task = Task::new(
            TaskKind::Http,
            Source::Urls {
                urls: vec![url.to_owned()],
            },
            "",
            settings.storage.download_directory.clone(),
            osprey_domain::QueueId::default_queue(),
        );
        HttpEngine::new()
            .probe(&task, settings, secrets.clone(), clients)
            .await
    }
}

#[async_trait::async_trait]
impl Transfer for HttpEngine {
    fn kinds(&self) -> &'static [TaskKind] {
        &[TaskKind::Http, TaskKind::Metalink]
    }

    async fn run(&self, ctx: TransferContext) -> TransferOutcome {
        let primary = ctx.task.source.primary_url().unwrap_or("").to_owned();
        match run_inner(self, ctx).await {
            Ok(outcome) => outcome,
            Err(mut e) => {
                if e.source_url.is_none() {
                    e.source_url = Some(redact(&primary));
                }
                e.message = redact(&e.message);
                TransferOutcome::Failed(e)
            }
        }
    }

    async fn probe(
        &self,
        task: &Task,
        settings: Arc<Settings>,
        secrets: TransferSecrets,
        clients: Arc<ClientFactory>,
    ) -> Result<ResolvedMetadata, TaskError> {
        let sources = resolve_sources(task, &clients, None).await?;
        let client = clients.client(&client_profile(task, &settings, &secrets))?;
        let template = RequestTemplate::build(task, &secrets)?;
        let (mirrors, reference, _) = if sources.urls.len() > 1 {
            mirrors::probe_all(&client, &template, &sources.urls, self.tuning.probe_timeout).await?
        } else {
            let p =
                probe::probe(&client, &template, &sources.urls[0], DIALOG_PROBE_TIMEOUT).await?;
            (MirrorSet::single(sources.urls[0].clone()), p, Vec::new())
        };
        let mut meta = reference.metadata;
        if let Some(ml) = &sources.metalink {
            meta.name = Some(ml.name.clone());
            if meta.total.is_none() {
                meta.total = ml.size;
            }
        }
        if mirrors.is_multi() {
            meta.final_url = Some(redact(mirrors.url(0)));
        }
        Ok(meta)
    }
}

// -------------------------------------------------------------------------------------------
// sources
// -------------------------------------------------------------------------------------------

struct Sources {
    urls: Vec<String>,
    metalink: Option<MetalinkFile>,
}

/// Turn the task's `Source` into a list of HTTP(S) URLs, fetching and parsing a metalink
/// document when needed.
async fn resolve_sources(
    task: &Task,
    clients: &ClientFactory,
    sink: Option<&dyn ProgressSink>,
) -> Result<Sources, TaskError> {
    match &task.source {
        Source::Urls { urls } if !urls.is_empty() => {
            for u in urls {
                parse_http_url(u)?;
            }
            Ok(Sources {
                urls: urls.clone(),
                metalink: None,
            })
        }
        Source::Metalink { url, document } => {
            let doc = match (document, url) {
                (Some(d), _) => d.clone(),
                (None, Some(u)) => fetch_metalink(clients, u).await?,
                (None, None) => {
                    return Err(TaskError::new(
                        ErrorKind::InvalidUrl,
                        "metalink task has neither a URL nor a document",
                    ))
                }
            };
            let ml = metalink::parse(&doc)?;
            if ml.file_count > 1 {
                // Multi-file metalinks are not expanded here; see README "Limitations".
                if let Some(s) = sink {
                    s.log(
                        LogLevel::Warn,
                        "metalink.multi_file",
                        format!(
                            "metalink lists {} files; only the first ({}) is downloaded",
                            ml.file_count, ml.name
                        ),
                    );
                }
            }
            for u in &ml.urls {
                parse_http_url(&u.url)?;
            }
            Ok(Sources {
                urls: ml.mirror_urls(),
                metalink: Some(ml),
            })
        }
        _ => Err(TaskError::new(
            ErrorKind::InvalidUrl,
            "source is not an HTTP download",
        )),
    }
}

async fn fetch_metalink(clients: &ClientFactory, url: &str) -> Result<String, TaskError> {
    parse_http_url(url)?;
    let client = clients.default_client()?;
    let resp = client
        .get(url)
        .timeout(DIALOG_PROBE_TIMEOUT)
        .send()
        .await
        .map_err(|e| crate::classify::classify_reqwest(&e, url))?;
    if !resp.status().is_success() {
        return Err(
            TaskError::from_http_status(resp.status().as_u16(), url).with_source(redact(url))
        );
    }
    let too_big = resp
        .headers()
        .get(http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .map(|n| n > metalink::MAX_DOCUMENT_BYTES)
        .unwrap_or(false);
    if too_big {
        return Err(
            TaskError::new(ErrorKind::ParseError, "metalink document too large")
                .with_source(redact(url)),
        );
    }
    let body = resp
        .bytes()
        .await
        .map_err(|e| crate::classify::classify_reqwest(&e, url))?;
    if body.len() > metalink::MAX_DOCUMENT_BYTES {
        return Err(
            TaskError::new(ErrorKind::ParseError, "metalink document too large")
                .with_source(redact(url)),
        );
    }
    String::from_utf8(body.to_vec()).map_err(|_| {
        TaskError::new(ErrorKind::ParseError, "metalink document is not UTF-8")
            .with_source(redact(url))
    })
}

// -------------------------------------------------------------------------------------------
// the run
// -------------------------------------------------------------------------------------------

async fn run_inner(
    engine: &HttpEngine,
    ctx: TransferContext,
) -> Result<TransferOutcome, TaskError> {
    let TransferContext {
        task,
        checkpoint,
        control,
        sink,
        settings,
        secrets,
        limiter,
        clients,
        ..
    } = ctx;

    if control.should_stop() {
        return Ok(control.stop_outcome());
    }
    sink.state(TaskState::Connecting, None);
    let sources = resolve_sources(&task, &clients, Some(sink.as_ref())).await?;
    let primary = sources.urls[0].clone();
    let primary_url = parse_http_url(&primary)?;
    let host = primary_url
        .host_str()
        .unwrap_or("unknown")
        .to_ascii_lowercase();
    engine.limits.configure(
        settings.network.max_connections_per_host as usize,
        settings.network.max_total_connections as usize,
    );
    let client = clients.client(&client_profile(&task, &settings, &secrets))?;
    let template = RequestTemplate::build(&task, &secrets)?;
    let backoff = BackoffPolicy::from_settings(&settings.network);
    let backoff = match task.options.max_retries {
        Some(n) => BackoffPolicy {
            max_retries: n,
            ..backoff
        },
        None => backoff,
    };

    // --- probe ------------------------------------------------------------------------------
    let (mirrors, reference) = probe_with_retry(
        engine,
        &control,
        sink.as_ref(),
        &client,
        &template,
        &sources.urls,
        &settings,
        &backoff,
    )
    .await?;
    let total = reference
        .total()
        .or(sources.metalink.as_ref().and_then(|m| m.size));
    let mut metadata = reference.metadata.clone();
    if let Some(ml) = &sources.metalink {
        metadata.name = Some(ml.name.clone());
        metadata.total = total;
    }
    sink.metadata(metadata.clone());
    let ranges_supported = reference.resumable() && total.is_some();
    sink.stat(EngineStat::RangeSupport {
        supported: ranges_supported,
    });
    sink.log(
        LogLevel::Info,
        "http.probe",
        redact(&format!(
            "{} size={} ranges={} type={} server={} {}",
            reference.final_url,
            total.map(|t| t.to_string()).unwrap_or_else(|| "?".into()),
            ranges_supported,
            reference.content_type.as_deref().unwrap_or("?"),
            metadata.server.as_deref().unwrap_or("?"),
            metadata.http_version.as_deref().unwrap_or("")
        )),
    );

    // --- expected content ---------------------------------------------------------------------
    let expect_binary = !url_is_html_page(&primary) && !reference.has_disposition;
    if expect_binary && reference.is_html() {
        return Err(TaskError::new(
            ErrorKind::UnexpectedContent,
            "the server sent an HTML page instead of the file",
        )
        .with_detail(format!(
            "content-type {}",
            reference.content_type.as_deref().unwrap_or("")
        ))
        .with_source(redact(&primary)));
    }

    // --- paths ---------------------------------------------------------------------------------
    let name = choose_name(&task, &reference, sources.metalink.as_ref());
    let final_path = task.directory.join(&name);
    ensure_within(&task.directory, &final_path)?;
    let suffix = &settings.storage.temp_suffix;
    let default_part = task.directory.join(format!("{name}{suffix}"));

    // --- resume decision -------------------------------------------------------------------
    let previous: Option<SegmentMap> = checkpoint
        .as_ref()
        .and_then(Checkpoint::as_segments)
        .cloned()
        .or_else(|| task.segment_map.clone());
    let (plan, part_path, fresh) = match previous {
        Some(map) if map.total.is_some() && !map.segments.is_empty() => {
            validate_resume(&map, &reference, total, !mirrors.is_multi(), &primary)?;
            let part = map
                .part_path
                .clone()
                .filter(|p| ensure_within(&task.directory, p).is_ok())
                .unwrap_or_else(|| default_part.clone());
            if !ranges_supported {
                sink.log(
                    LogLevel::Warn,
                    "http.range_unsupported",
                    "server no longer honours ranges; starting over with one connection".into(),
                );
                (
                    Plan::single(total, settings.network.min_segment_size),
                    default_part.clone(),
                    true,
                )
            } else if !part.exists() {
                sink.log(
                    LogLevel::Warn,
                    "http.resume",
                    format!("part file {} is missing; starting over", part.display()),
                );
                (
                    fresh_plan(&task, &settings, &control, total),
                    default_part.clone(),
                    true,
                )
            } else {
                let plan = Plan::from_map(&map, settings.network.min_segment_size);
                sink.log(
                    LogLevel::Info,
                    "http.resume",
                    format!(
                        "resuming: {} of {} bytes committed in {} segments",
                        plan.committed_bytes(),
                        total.unwrap_or(0),
                        plan.segments.len()
                    ),
                );
                (plan, part, false)
            }
        }
        _ => {
            let plan = if ranges_supported {
                fresh_plan(&task, &settings, &control, total)
            } else {
                if total.is_some() {
                    sink.log(
                        LogLevel::Info,
                        "http.range_unsupported",
                        "server does not support byte ranges; single connection, no resume".into(),
                    );
                }
                Plan::single(total, settings.network.min_segment_size)
            };
            (plan, default_part.clone(), true)
        }
    };
    ensure_within(&task.directory, &part_path)?;

    // --- writer ----------------------------------------------------------------------------
    let writer = open_writer(&task, &settings, &part_path, total, fresh).await?;
    control.counters.set_downloaded(plan.written_bytes());
    let initial_target = plan
        .segments
        .iter()
        .filter(|s| s.status != crate::plan::SegStatus::Done)
        .count()
        .max(1)
        .min(requested_connections(&task, &settings, &control));

    let shared = Arc::new(RunShared {
        task: task.clone(),
        settings: settings.clone(),
        control: control.clone(),
        sink: sink.clone(),
        limiter,
        client,
        template,
        writer: writer.clone(),
        plan: Mutex::new(plan),
        mirrors: Mutex::new(mirrors),
        limits: engine.limits.clone(),
        host,
        tuning: engine.tuning.clone(),
        backoff,
        ranges: AtomicBool::new(ranges_supported),
        expected_total: total,
        etag: metadata.etag.clone(),
        last_modified: metadata.last_modified.clone(),
        validate_validators: sources.urls.len() <= 1,
        expect_binary,
        part_path: part_path.clone(),
        throttle_events: AtomicU64::new(0),
        reset_events: AtomicU64::new(0),
    });

    sink.state(TaskState::Downloading, None);
    let mut result = Controller::new(shared.clone(), initial_target).run().await;
    if matches!(result, RunResult::RangeUnsupported) {
        // The probe lied (or a middlebox strips Range): degrade to one connection from 0.
        sink.stat(EngineStat::RangeSupport { supported: false });
        sink.log(
            LogLevel::Warn,
            "http.range_unsupported",
            "server ignored a range request; restarting with a single connection".into(),
        );
        let discarded = control.counters.downloaded();
        if discarded > 0 {
            sink.stat(EngineStat::BytesDiscarded { bytes: discarded });
        }
        shared
            .ranges
            .store(false, std::sync::atomic::Ordering::Relaxed);
        shared.writer.truncate(0).await?;
        *shared.plan.lock() = Plan::single(total, settings.network.min_segment_size);
        control.counters.set_downloaded(0);
        result = Controller::new(shared.clone(), 1).run().await;
    }

    match result {
        RunResult::Completed => {
            finalize(
                &shared,
                &sink,
                &control,
                &settings,
                &final_path,
                &primary,
                sources.metalink.as_ref(),
            )
            .await
        }
        RunResult::Stopped => {
            let _ = writer.close().await;
            Ok(control.stop_outcome())
        }
        RunResult::Failed(e) => {
            let _ = writer.close().await;
            Err(e)
        }
        RunResult::RangeUnsupported => {
            let _ = writer.close().await;
            Err(TaskError::new(
                ErrorKind::RangeNotSupported,
                "server ignores range requests",
            )
            .with_source(redact(&primary)))
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn probe_with_retry(
    engine: &HttpEngine,
    control: &TransferControl,
    sink: &dyn ProgressSink,
    client: &reqwest::Client,
    template: &RequestTemplate,
    urls: &[String],
    settings: &Settings,
    backoff: &BackoffPolicy,
) -> Result<(MirrorSet, ProbeResult), TaskError> {
    let single_timeout = Duration::from_secs(settings.network.read_timeout_seconds.max(5) as u64);
    let mut attempt = 0u32;
    loop {
        let r: Result<(MirrorSet, ProbeResult), TaskError> = if urls.len() > 1 {
            match mirrors::probe_all(client, template, urls, engine.tuning.probe_timeout).await {
                Ok((set, reference, rejected)) => {
                    for (i, e) in rejected {
                        sink.log(
                            LogLevel::Warn,
                            "mirror.switch",
                            redact(&format!(
                                "mirror {i} ({}) not used: {}",
                                urls.get(i).map(String::as_str).unwrap_or("?"),
                                e.message
                            )),
                        );
                    }
                    if set.healthy_count() == 0 {
                        Err(set.exhausted_error())
                    } else {
                        Ok((set, reference))
                    }
                }
                Err(e) => Err(e),
            }
        } else {
            probe::probe(client, template, &urls[0], single_timeout)
                .await
                .map(|p| (MirrorSet::single(data_url(&urls[0], &p.final_url)), p))
        };
        match r {
            Ok(v) => return Ok(v),
            Err(e) => {
                attempt += 1;
                let retry = matches!(e.class(), FailureClass::Transient | FailureClass::Throttled);
                let delay = if retry {
                    backoff.delay_for_with_hint(
                        attempt,
                        e.class(),
                        e.retry_after_ms.map(Duration::from_millis),
                    )
                } else {
                    None
                };
                let Some(delay) = delay else {
                    return Err(e);
                };
                sink.stat(EngineStat::Retry);
                sink.log(
                    LogLevel::Info,
                    "http.probe",
                    redact(&format!(
                        "probe attempt {attempt} failed ({}); retrying in {:.1}s",
                        e.message,
                        delay.as_secs_f64()
                    )),
                );
                sink.state(TaskState::Retrying, Some(e.message.clone()));
                tokio::select! {
                    _ = control.stopped() => return Err(TaskError::cancelled()),
                    _ = tokio::time::sleep(delay) => {}
                }
                sink.state(TaskState::Connecting, None);
            }
        }
    }
}

fn requested_connections(task: &Task, settings: &Settings, control: &TransferControl) -> usize {
    let user = control.max_connections() as usize;
    let n = if user > 0 {
        user
    } else {
        task.options
            .max_connections
            .map(|n| n as usize)
            .filter(|n| *n > 0)
            .unwrap_or(settings.network.connections_per_task as usize)
    };
    n.clamp(1, crate::hostlimits::ABSOLUTE_MAX_CONNECTIONS)
        .min(settings.network.max_connections_per_host.max(1) as usize)
}

fn fresh_plan(
    task: &Task,
    settings: &Settings,
    control: &TransferControl,
    total: Option<u64>,
) -> Plan {
    match total {
        Some(t) => {
            let n = initial_segment_count(
                t,
                requested_connections(task, settings, control),
                settings.network.min_segment_size,
            );
            Plan::initial(t, n, settings.network.min_segment_size)
        }
        None => Plan::single(None, settings.network.min_segment_size),
    }
}

fn validate_resume(
    map: &SegmentMap,
    reference: &ProbeResult,
    total: Option<u64>,
    check_validators: bool,
    url: &str,
) -> Result<(), TaskError> {
    let changed = |what: &str, before: &str, now: &str| {
        Err(TaskError::new(
            ErrorKind::SourceChanged,
            "the file changed on the server since the download started",
        )
        .with_detail(format!("{what}: was {before}, now {now}"))
        .with_source(redact(url)))
    };
    if map.total != total {
        return changed(
            "size",
            &map.total.map(|t| t.to_string()).unwrap_or_default(),
            &total.map(|t| t.to_string()).unwrap_or_else(|| "?".into()),
        );
    }
    if check_validators {
        if let (Some(a), Some(b)) = (&map.etag, &reference.metadata.etag) {
            if a != b {
                return changed("etag", a, b);
            }
        }
        if let (Some(a), Some(b)) = (&map.last_modified, &reference.metadata.last_modified) {
            if a != b {
                return changed("last-modified", a, b);
            }
        }
    }
    Ok(())
}

/// Which name the file gets: a locked name wins; then the metalink; then an explicit
/// `Content-Disposition`; then whatever the task already carries; then the probe's guess.
fn choose_name(task: &Task, reference: &ProbeResult, metalink: Option<&MetalinkFile>) -> String {
    let probed = reference.metadata.name.clone().unwrap_or_default();
    let raw = if task.name_locked && !task.name.trim().is_empty() {
        task.name.clone()
    } else if let Some(ml) = metalink {
        ml.name.clone()
    } else if reference.has_disposition && !probed.is_empty() {
        probed
    } else if !task.name.trim().is_empty() {
        task.name.clone()
    } else if !probed.is_empty() {
        probed
    } else {
        "download".to_owned()
    };
    sanitize_filename(&raw)
}

/// A URL whose path ends in `.html`/`.htm` is a page by intent; HTML is then not a surprise.
fn url_is_html_page(url: &str) -> bool {
    url::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.path_segments()
                .and_then(|mut s| s.next_back().map(|l| l.to_ascii_lowercase()))
        })
        .map(|leaf| leaf.ends_with(".html") || leaf.ends_with(".htm"))
        .unwrap_or(false)
}

async fn open_writer(
    task: &Task,
    settings: &Settings,
    part_path: &Path,
    total: Option<u64>,
    fresh: bool,
) -> Result<FileWriter, TaskError> {
    if fresh {
        // A fresh start must not inherit bytes from an older attempt.
        match tokio::fs::remove_file(part_path).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(TaskError::from_io_ctx(
                    &e,
                    &format!("remove {}", part_path.display()),
                    IoContext::Disk,
                ))
            }
        }
    }
    let preallocate = task
        .options
        .preallocate
        .unwrap_or(settings.storage.preallocate);
    let sparse = task.options.sparse.unwrap_or(settings.storage.sparse_files);
    FileWriter::open(
        part_path,
        OpenOptionsExt {
            preallocate: if fresh && preallocate { total } else { None },
            sparse,
            in_flight_bytes: None,
            expected_root: Some(task.directory.clone()),
        },
    )
    .await
}

async fn finalize(
    shared: &Arc<RunShared>,
    sink: &Arc<dyn ProgressSink>,
    control: &Arc<TransferControl>,
    settings: &Settings,
    final_path: &Path,
    primary: &str,
    metalink: Option<&MetalinkFile>,
) -> Result<TransferOutcome, TaskError> {
    let bytes = shared.plan.lock().written_bytes();
    shared.writer.flush(FlushLevel::Full).await?;
    shared.writer.clone().close().await?;
    let part = shared.part_path.clone();

    if let Some(expected) = metalink.and_then(MetalinkFile::best_hash) {
        sink.state(
            TaskState::Verifying,
            Some(expected.algorithm.as_str().to_owned()),
        );
        sink.log(
            LogLevel::Info,
            "http.verify",
            format!(
                "verifying {} from the metalink",
                expected.algorithm.as_str()
            ),
        );
        osprey_runtime::checksum::verify_file(&part, expected, control.cancel.clone())
            .await
            .map_err(|e| e.with_source(redact(primary)))?;
    }

    if settings.storage.quarantine_downloads {
        if let Err(e) = set_quarantine(&part, "Osprey", Some(primary)) {
            sink.log(
                LogLevel::Warn,
                "http.quarantine",
                format!("could not set the quarantine attribute: {e}"),
            );
        }
    }

    let target = if final_path.exists() {
        unique_path(final_path)
    } else {
        final_path.to_path_buf()
    };
    ensure_within(&shared.task.directory, &target)?;
    tokio::fs::rename(&part, &target).await.map_err(|e| {
        TaskError::from_io_ctx(
            &e,
            &format!("rename to {}", target.display()),
            IoContext::Disk,
        )
    })?;
    sink.log(
        LogLevel::Info,
        "http.complete",
        format!("{bytes} bytes -> {}", target.display()),
    );
    Ok(TransferOutcome::Completed {
        file_path: target,
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_page_detection() {
        assert!(url_is_html_page("https://h/a/index.html?x=1"));
        assert!(url_is_html_page("https://h/page.HTM"));
        assert!(!url_is_html_page("https://h/file.zip"));
        assert!(!url_is_html_page("https://h/download?id=1"));
    }

    #[test]
    fn name_choice() {
        let mut task = Task::new(
            TaskKind::Http,
            Source::Urls {
                urls: vec!["https://h/x.bin".into()],
            },
            "x.bin",
            std::path::PathBuf::from("/tmp"),
            osprey_domain::QueueId::default_queue(),
        );
        let mut reference = ProbeResult::default();
        reference.metadata.name = Some("report.pdf".into());
        assert_eq!(choose_name(&task, &reference, None), "x.bin");
        reference.has_disposition = true;
        assert_eq!(choose_name(&task, &reference, None), "report.pdf");
        task.name_locked = true;
        assert_eq!(choose_name(&task, &reference, None), "x.bin");
    }
}
