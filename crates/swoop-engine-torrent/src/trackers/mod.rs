//! Per-torrent tracker bookkeeping.
//!
//! librqbit announces to trackers internally but exposes no per-tracker state (no status, no
//! seeder/leecher counts, no errors). We therefore keep our own [`TrackerRegistry`] per torrent
//! and run light-weight announce *probes* (`numwant=0`, same peer id and port as librqbit so
//! trackers see one peer) to learn health, latency and swarm sizes. Peer discovery itself
//! stays with librqbit.
//!
//! Private torrents (BEP-27) never get trackers added from settings, curated lists or the user.

pub mod bencode;
pub mod http;
pub mod probe;
pub mod udp;

use parking_lot::Mutex;
use probe::{AnnounceEvent, AnnounceRequest, ProbeOutcome};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use swoop_domain::torrent::TrackerStatus;
use swoop_domain::{ErrorKind, LogLevel, Millis, TaskError};
use swoop_runtime::engine::ProgressSink;
use swoop_runtime::redact::redact;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

/// Trackers are never probed more often than this unless the user overrides the interval.
pub const MIN_PROBE_INTERVAL: Duration = Duration::from_secs(300);
/// Failed trackers back off up to this.
pub const MAX_BACKOFF: Duration = Duration::from_secs(3600);
/// Consecutive failures after which a tracker is reported `dead` (it is still retried hourly).
pub const DEAD_AFTER_FAILURES: u32 = 3;

pub const HEALTH_WORKING: &str = "working";
pub const HEALTH_UPDATING: &str = "updating";
pub const HEALTH_ERROR: &str = "error";
pub const HEALTH_DISABLED: &str = "disabled";
pub const HEALTH_DEAD: &str = "dead";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackerSource {
    /// Announce / announce-list of the `.torrent`.
    Metainfo,
    /// `tr=` parameters of a magnet link.
    Magnet,
    /// Added by the user (persisted in the checkpoint as `extra_trackers`).
    User,
    /// `settings.torrent.additional_trackers` or a refreshed curated list.
    Settings,
}

#[derive(Clone, Debug)]
pub struct TrackerRow {
    pub status: TrackerStatus,
    pub source: TrackerSource,
    announced_started: bool,
    force: bool,
    in_flight: bool,
}

struct Inner {
    private: bool,
    rows: Vec<TrackerRow>,
    /// Bumped whenever the *enabled set* changes; the run loop compares it against what
    /// librqbit was given to decide whether the torrent must be re-added.
    generation: u64,
}

pub struct TrackerRegistry {
    inner: Mutex<Inner>,
    wake: Notify,
}

impl Default for TrackerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TrackerRegistry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                private: false,
                rows: Vec::new(),
                generation: 0,
            }),
            wake: Notify::new(),
        }
    }

    /// (Re)build the rows for a run. Probe results of URLs that survive are kept.
    pub fn reset(
        &self,
        private: bool,
        metainfo: &[(String, u32)],
        magnet: &[String],
        disabled: &[String],
        extra: &[String],
        settings: &[String],
    ) {
        let mut g = self.inner.lock();
        let old: Vec<TrackerRow> = std::mem::take(&mut g.rows);
        g.private = private;
        let mut rows: Vec<TrackerRow> = Vec::new();
        let push = |rows: &mut Vec<TrackerRow>, url: &str, tier: u32, source: TrackerSource| {
            let url = url.trim();
            if url.is_empty() || rows.iter().any(|r| r.status.url == url) {
                return;
            }
            let mut row = old
                .iter()
                .find(|r| r.status.url == url)
                .cloned()
                .unwrap_or_else(|| new_row(url, tier, source));
            row.status.tier = tier;
            row.source = source;
            row.in_flight = false;
            rows.push(row);
        };
        for (url, tier) in metainfo {
            push(&mut rows, url, *tier, TrackerSource::Metainfo);
        }
        for url in magnet {
            push(&mut rows, url, 0, TrackerSource::Magnet);
        }
        let next_tier =
            |rows: &[TrackerRow]| rows.iter().map(|r| r.status.tier + 1).max().unwrap_or(0);
        for url in extra {
            let t = next_tier(&rows);
            push(&mut rows, url, t, TrackerSource::User);
        }
        if !private {
            for url in settings {
                let t = next_tier(&rows);
                push(&mut rows, url, t, TrackerSource::Settings);
            }
        }
        for row in rows.iter_mut() {
            let enabled = !disabled.iter().any(|d| d == &row.status.url);
            row.status.enabled = enabled;
            if !enabled {
                row.status.health = HEALTH_DISABLED.into();
            } else if row.status.health == HEALTH_DISABLED {
                row.status.health = HEALTH_UPDATING.into();
            }
        }
        g.rows = rows;
        g.generation += 1;
        self.wake.notify_one();
    }

    pub fn is_private(&self) -> bool {
        self.inner.lock().private
    }

    pub fn statuses(&self) -> Vec<TrackerStatus> {
        self.inner
            .lock()
            .rows
            .iter()
            .map(|r| r.status.clone())
            .collect()
    }

    /// Enabled URLs grouped by tier, in tier order — the announce-list handed to librqbit.
    pub fn enabled_tiers(&self) -> Vec<Vec<String>> {
        let g = self.inner.lock();
        let tiers: BTreeSet<u32> = g
            .rows
            .iter()
            .filter(|r| r.status.enabled)
            .map(|r| r.status.tier)
            .collect();
        tiers
            .into_iter()
            .map(|t| {
                g.rows
                    .iter()
                    .filter(|r| r.status.enabled && r.status.tier == t)
                    .map(|r| r.status.url.clone())
                    .collect()
            })
            .collect()
    }

    pub fn generation(&self) -> u64 {
        self.inner.lock().generation
    }

    pub fn disabled_urls(&self) -> Vec<String> {
        self.inner
            .lock()
            .rows
            .iter()
            .filter(|r| !r.status.enabled)
            .map(|r| r.status.url.clone())
            .collect()
    }

    pub fn user_urls(&self) -> Vec<String> {
        self.inner
            .lock()
            .rows
            .iter()
            .filter(|r| r.source == TrackerSource::User)
            .map(|r| r.status.url.clone())
            .collect()
    }

    /// Largest seeder / leecher counts reported by any tracker (trackers describe overlapping
    /// swarms, so summing would over-count).
    pub fn swarm_estimate(&self) -> (u32, u32) {
        let g = self.inner.lock();
        let seeders = g.rows.iter().filter_map(|r| r.status.seeders).max();
        let leechers = g.rows.iter().filter_map(|r| r.status.leechers).max();
        (seeders.unwrap_or(0), leechers.unwrap_or(0))
    }

    /// Add trackers. Returns how many were new. Private torrents refuse additions.
    pub fn add(&self, urls: &[String], source: TrackerSource) -> Result<usize, TaskError> {
        let mut g = self.inner.lock();
        if g.private && source != TrackerSource::Metainfo {
            return Err(private_error());
        }
        let mut added = 0;
        for url in urls {
            let url = validate_tracker_url(url)?;
            if g.rows.iter().any(|r| r.status.url == url) {
                continue;
            }
            let tier = g.rows.iter().map(|r| r.status.tier + 1).max().unwrap_or(0);
            let mut row = new_row(&url, tier, source);
            row.force = true;
            g.rows.push(row);
            added += 1;
        }
        if added > 0 {
            g.generation += 1;
            self.wake.notify_one();
        }
        Ok(added)
    }

    pub fn remove(&self, url: &str) -> Result<(), TaskError> {
        let mut g = self.inner.lock();
        let before = g.rows.len();
        g.rows.retain(|r| r.status.url != url);
        if g.rows.len() == before {
            return Err(TaskError::new(
                ErrorKind::NotFound,
                format!("tracker {} is not attached to this torrent", redact(url)),
            ));
        }
        g.generation += 1;
        self.wake.notify_one();
        Ok(())
    }

    pub fn set_enabled(&self, url: &str, enabled: bool) -> Result<(), TaskError> {
        let mut g = self.inner.lock();
        let row = g
            .rows
            .iter_mut()
            .find(|r| r.status.url == url)
            .ok_or_else(|| {
                TaskError::new(
                    ErrorKind::NotFound,
                    format!("tracker {} is not attached to this torrent", redact(url)),
                )
            })?;
        if row.status.enabled == enabled {
            return Ok(());
        }
        row.status.enabled = enabled;
        row.status.health = if enabled {
            HEALTH_UPDATING
        } else {
            HEALTH_DISABLED
        }
        .into();
        row.force = enabled;
        g.generation += 1;
        self.wake.notify_one();
        Ok(())
    }

    /// Probe every enabled tracker as soon as possible.
    pub fn force_all(&self) {
        let mut g = self.inner.lock();
        for r in g.rows.iter_mut().filter(|r| r.status.enabled) {
            r.force = true;
        }
        self.wake.notify_one();
    }

    /// Enabled trackers due for a probe; marks them in flight. Returns `(url, first_announce)`.
    fn take_due(&self, now: Millis) -> Vec<(String, bool)> {
        let mut g = self.inner.lock();
        let mut due = Vec::new();
        for r in g.rows.iter_mut() {
            if !r.status.enabled || r.in_flight {
                continue;
            }
            let ready = r.force || r.status.next_announce_at.is_none_or(|t| t.0 <= now.0);
            if ready {
                r.in_flight = true;
                r.force = false;
                r.status.health = HEALTH_UPDATING.into();
                due.push((r.status.url.clone(), !r.announced_started));
            }
        }
        due
    }

    /// Store a probe result. Returns `(previous health, new health)` for logging.
    fn record(
        &self,
        url: &str,
        outcome: &ProbeOutcome,
        interval_override: Option<Duration>,
    ) -> Option<(String, String)> {
        let mut g = self.inner.lock();
        let row = g.rows.iter_mut().find(|r| r.status.url == url)?;
        row.in_flight = false;
        let previous = row.status.health.clone();
        let now = Millis::now();
        row.status.last_announce_at = Some(now);
        row.status.latency_ms =
            Some(u32::try_from(outcome.latency.as_millis()).unwrap_or(u32::MAX));
        match &outcome.result {
            Ok(resp) => {
                row.announced_started = true;
                row.status.consecutive_failures = 0;
                row.status.last_error = None;
                row.status.seeders = resp.seeders;
                row.status.leechers = resp.leechers;
                let announced = resp
                    .interval
                    .max(resp.min_interval)
                    .map(Duration::from_secs);
                let interval = interval_override.unwrap_or_else(|| {
                    announced
                        .unwrap_or(MIN_PROBE_INTERVAL)
                        .max(MIN_PROBE_INTERVAL)
                });
                row.status.next_announce_at = Some(after(now, interval));
                row.status.health = HEALTH_WORKING.into();
            }
            Err(e) => {
                row.status.consecutive_failures = row.status.consecutive_failures.saturating_add(1);
                row.status.last_error = Some(e.message.clone());
                let failures = row.status.consecutive_failures;
                let backoff = if failures >= DEAD_AFTER_FAILURES {
                    MAX_BACKOFF
                } else {
                    let base = interval_override.unwrap_or(MIN_PROBE_INTERVAL);
                    (base * 2u32.saturating_pow(failures.saturating_sub(1))).min(MAX_BACKOFF)
                };
                row.status.next_announce_at = Some(after(now, backoff));
                row.status.health = if failures >= DEAD_AFTER_FAILURES {
                    HEALTH_DEAD
                } else {
                    HEALTH_ERROR
                }
                .into();
            }
        }
        if !row.status.enabled {
            row.status.health = HEALTH_DISABLED.into();
        }
        Some((previous, row.status.health.clone()))
    }
}

fn after(now: Millis, d: Duration) -> Millis {
    now.saturating_add_ms(i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn new_row(url: &str, tier: u32, source: TrackerSource) -> TrackerRow {
    TrackerRow {
        status: TrackerStatus {
            url: url.to_owned(),
            tier,
            enabled: true,
            last_announce_at: None,
            next_announce_at: None,
            seeders: None,
            leechers: None,
            latency_ms: None,
            last_error: None,
            health: HEALTH_UPDATING.into(),
            consecutive_failures: 0,
        },
        source,
        announced_started: false,
        force: true,
        in_flight: false,
    }
}

pub fn private_error() -> TaskError {
    TaskError::new(
        ErrorKind::PermissionDenied,
        "private torrent: trackers cannot be added or replaced (BEP-27)",
    )
}

/// Accept `http`, `https` and `udp` tracker URLs; reject everything else (including `ws`).
pub fn validate_tracker_url(url: &str) -> Result<String, TaskError> {
    let trimmed = url.trim();
    let parsed = url::Url::parse(trimmed).map_err(|e| {
        TaskError::new(
            ErrorKind::InvalidUrl,
            format!("invalid tracker URL {}: {e}", redact(trimmed)),
        )
    })?;
    match parsed.scheme() {
        "http" | "https" => {}
        "udp" if parsed.port().is_some() => {}
        "udp" => {
            return Err(TaskError::new(
                ErrorKind::InvalidUrl,
                "udp tracker URL needs an explicit port",
            ))
        }
        "ws" | "wss" => {
            return Err(TaskError::new(
                ErrorKind::UnsupportedScheme,
                "WebSocket (WebTorrent) trackers are not supported",
            ))
        }
        other => {
            return Err(TaskError::new(
                ErrorKind::UnsupportedScheme,
                format!("unsupported tracker scheme {other:?}"),
            ))
        }
    }
    if parsed.host_str().is_none() {
        return Err(TaskError::new(
            ErrorKind::InvalidUrl,
            "tracker URL has no host",
        ));
    }
    Ok(trimmed.to_owned())
}

/// Parse a newline-separated tracker list (comments and blank lines ignored, invalid URLs
/// skipped, duplicates removed).
pub fn parse_tracker_list(text: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Ok(url) = validate_tracker_url(line) {
            if seen.insert(url.clone()) {
                out.push(url);
            }
        }
    }
    out
}

/// Counters the probes report to trackers; read fresh before every announce.
#[derive(Clone, Copy, Debug, Default)]
pub struct AnnounceStats {
    pub uploaded: u64,
    pub downloaded: u64,
    pub left: u64,
}

pub trait AnnounceStatsSource: Send + Sync {
    fn announce_stats(&self) -> AnnounceStats;
}

/// Everything the probe loop needs for one torrent.
pub struct ProbeContext {
    pub registry: Arc<TrackerRegistry>,
    pub http: reqwest::Client,
    pub info_hash: [u8; 20],
    pub peer_id: [u8; 20],
    pub port: u16,
    pub stats: Arc<dyn AnnounceStatsSource>,
    /// `settings.torrent.announce_interval_seconds` when non-zero.
    pub interval_override: Option<Duration>,
    pub sink: Arc<dyn ProgressSink>,
    pub cancel: CancellationToken,
}

/// Run probes for one torrent until `cancel` fires. Wakes every second or when the registry
/// changes; probes due trackers concurrently.
pub fn spawn_probe_loop(ctx: ProbeContext) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let key: u32 = rand::random();
        loop {
            let due = ctx.registry.take_due(Millis::now());
            if !due.is_empty() {
                let stats = ctx.stats.announce_stats();
                let probes = due.into_iter().map(|(url, first)| {
                    let request = AnnounceRequest {
                        info_hash: ctx.info_hash,
                        peer_id: ctx.peer_id,
                        port: ctx.port,
                        uploaded: stats.uploaded,
                        downloaded: stats.downloaded,
                        left: stats.left,
                        event: if first {
                            AnnounceEvent::Started
                        } else {
                            AnnounceEvent::None
                        },
                        key,
                        numwant: 0,
                    };
                    let http = ctx.http.clone();
                    async move {
                        let outcome = probe::probe(&url, &request, &http).await;
                        (url, outcome)
                    }
                });
                let results = tokio::select! {
                    r = futures::future::join_all(probes) => r,
                    _ = ctx.cancel.cancelled() => return,
                };
                for (url, outcome) in results {
                    if let Some((prev, next)) =
                        ctx.registry.record(&url, &outcome, ctx.interval_override)
                    {
                        log_transition(&ctx.sink, &url, &outcome, &prev, &next);
                    }
                }
            }
            tokio::select! {
                _ = ctx.cancel.cancelled() => return,
                _ = ctx.registry.wake.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            }
        }
    })
}

fn log_transition(
    sink: &Arc<dyn ProgressSink>,
    url: &str,
    outcome: &ProbeOutcome,
    prev: &str,
    next: &str,
) {
    let url = redact(url);
    match &outcome.result {
        Ok(resp) if prev != HEALTH_WORKING => sink.log(
            LogLevel::Info,
            "tracker.working",
            format!(
                "{url}: {} seeders, {} leechers ({} ms)",
                resp.seeders.unwrap_or(0),
                resp.leechers.unwrap_or(0),
                outcome.latency.as_millis()
            ),
        ),
        Ok(_) => {}
        Err(e) if next == HEALTH_DEAD && prev != HEALTH_DEAD => sink.log(
            LogLevel::Warn,
            "tracker.dead",
            format!("{url}: giving up after repeated failures: {}", e.message),
        ),
        Err(e) if next == HEALTH_ERROR && prev != HEALTH_ERROR => sink.log(
            LogLevel::Warn,
            "tracker.error",
            format!("{url}: {}", e.message),
        ),
        Err(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use probe::{AnnounceResponse, ProbeError};

    fn ok_outcome(seeders: u32) -> ProbeOutcome {
        ProbeOutcome {
            result: Ok(AnnounceResponse {
                interval: Some(60),
                seeders: Some(seeders),
                leechers: Some(1),
                ..Default::default()
            }),
            latency: Duration::from_millis(12),
        }
    }

    fn err_outcome() -> ProbeOutcome {
        ProbeOutcome {
            result: Err(ProbeError::transient("boom")),
            latency: Duration::from_millis(5),
        }
    }

    #[test]
    fn reset_merges_sources_and_respects_private() {
        let reg = TrackerRegistry::new();
        reg.reset(
            false,
            &[
                ("http://a/announce".into(), 0),
                ("http://b/announce".into(), 1),
            ],
            &["udp://m:1/announce".into()],
            &["http://b/announce".into()],
            &["http://user/announce".into()],
            &["http://settings/announce".into()],
        );
        let s = reg.statuses();
        assert_eq!(s.len(), 5);
        assert!(!s[1].enabled);
        assert_eq!(s[1].health, HEALTH_DISABLED);
        assert_eq!(reg.enabled_tiers().len(), 3); // tier 0 = a + magnet, then user, then settings
        assert_eq!(reg.disabled_urls(), vec!["http://b/announce".to_string()]);
        assert_eq!(reg.user_urls(), vec!["http://user/announce".to_string()]);

        let private = TrackerRegistry::new();
        private.reset(
            true,
            &[("https://p/announce?passkey=abc".into(), 0)],
            &[],
            &[],
            &[],
            &["http://settings/announce".into()],
        );
        assert_eq!(private.statuses().len(), 1);
        let err = private
            .add(&["http://x/announce".into()], TrackerSource::User)
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::PermissionDenied);
    }

    #[test]
    fn health_transitions_and_backoff() {
        let reg = TrackerRegistry::new();
        reg.reset(
            false,
            &[("http://a/announce".into(), 0)],
            &[],
            &[],
            &[],
            &[],
        );
        let due = reg.take_due(Millis::now());
        assert_eq!(due, vec![("http://a/announce".to_string(), true)]);
        assert!(reg.take_due(Millis::now()).is_empty(), "in flight");
        reg.record("http://a/announce", &ok_outcome(3), None);
        let s = &reg.statuses()[0];
        assert_eq!(s.health, HEALTH_WORKING);
        assert_eq!(s.seeders, Some(3));
        assert_eq!(s.latency_ms, Some(12));
        // 60 s interval is raised to the 5 min floor
        let gap = s.next_announce_at.unwrap().0 - s.last_announce_at.unwrap().0;
        assert!(gap >= 299_000, "{gap}");
        assert!(reg.take_due(Millis::now()).is_empty());

        reg.force_all();
        for i in 1..=3 {
            assert_eq!(reg.take_due(Millis::now()).len(), 1);
            reg.record(
                "http://a/announce",
                &err_outcome(),
                Some(Duration::from_secs(10)),
            );
            let s = &reg.statuses()[0];
            assert_eq!(s.consecutive_failures, i);
            assert_eq!(s.health, if i >= 3 { HEALTH_DEAD } else { HEALTH_ERROR });
            reg.force_all();
        }
        assert_eq!(reg.swarm_estimate(), (3, 1));
    }

    #[test]
    fn enable_disable_and_remove_bump_generation() {
        let reg = TrackerRegistry::new();
        reg.reset(
            false,
            &[("http://a/announce".into(), 0)],
            &[],
            &[],
            &[],
            &[],
        );
        let g0 = reg.generation();
        reg.set_enabled("http://a/announce", false).unwrap();
        assert!(reg.enabled_tiers().is_empty());
        assert_eq!(reg.statuses()[0].health, HEALTH_DISABLED);
        assert!(reg.generation() > g0);
        assert!(reg.set_enabled("http://nope", true).is_err());
        assert_eq!(
            reg.add(&["udp://t.example:80/announce".into()], TrackerSource::User)
                .unwrap(),
            1
        );
        assert_eq!(
            reg.add(&["udp://t.example:80/announce".into()], TrackerSource::User)
                .unwrap(),
            0
        );
        assert!(reg
            .add(&["wss://t.example/announce".into()], TrackerSource::User)
            .is_err());
        reg.remove("http://a/announce").unwrap();
        assert!(reg.remove("http://a/announce").is_err());
        assert_eq!(reg.statuses().len(), 1);
    }

    #[test]
    fn tracker_list_parsing() {
        let list = "# comment\nhttp://a/announce\n\nudp://b:6969/announce\nhttp://a/announce\nftp://no\nwss://ws.example/announce\n udp://noport/announce \n";
        assert_eq!(
            parse_tracker_list(list),
            vec![
                "http://a/announce".to_string(),
                "udp://b:6969/announce".to_string()
            ]
        );
    }
}
