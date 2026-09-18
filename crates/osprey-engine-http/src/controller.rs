//! The per-run controller: owns the worker set and every policy decision.
//!
//! * **Slots.** `target` is how many connections we want. Whenever `running < target` a slot
//!   is filled with the largest ready pending segment, or — when nothing is pending — by
//!   splitting the largest running segment (work-stealing) if both halves stay ≥ `min_segment`.
//! * **Adaptive concurrency.** Every `adapt_interval` the aggregate rate is sampled. Growth is
//!   an experiment: `target += 1`, wait `settle_samples`, keep the connection only if the
//!   aggregate rate improved by ≥ `grow_gain_threshold` (15 %), otherwise revert and cool
//!   down. Shrinking is immediate on 429/503/`Retry-After` (the server told us), on bursts
//!   of connection resets, and when the aggregate at *n* connections is no better than what
//!   *n/2* or fewer achieved earlier (per-client throttling: more connections only slice the
//!   same pipe thinner).
//! * **Endgame.** When nothing is pending, a segment more than `slow_segment_factor` slower
//!   than the median is split so a faster connection can take its tail.
//! * **Durability.** One barrier flush per `checkpoint_interval` (and on stop); `committed`
//!   only advances after it succeeds; the checkpoint is emitted afterwards.
//! * **Stop.** Workers are cancelled, the writer is flushed with a bounded timeout, a
//!   checkpoint is emitted only if that flush succeeded.

use crate::plan::SegStatus;
use crate::segment::{run_worker, FailureOrigin, WorkerOutcome, WorkerResult};
use crate::shared::RunShared;
use osprey_domain::checkpoint::Checkpoint;
use osprey_domain::events::LogLevel;
use osprey_domain::{ErrorKind, FailureClass, Progress, TaskError};
use osprey_runtime::disk::FlushLevel;
use osprey_runtime::engine::EngineStat;
use osprey_runtime::speed::SpeedMeter;
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub enum RunResult {
    Completed,
    /// Pause or cancel was requested; the caller reads `control.stop_outcome()`.
    Stopped,
    Failed(TaskError),
    /// Ranges turned out to be unsupported mid-run; the caller restarts in single mode.
    RangeUnsupported,
}

struct Adaptive {
    enabled: bool,
    target: usize,
    hard_max: usize,
    last_sample: Instant,
    last_bytes: u64,
    last_rate: f64,
    /// Smoothed aggregate rate observed while exactly `n` connections were running.
    rate_at: BTreeMap<usize, f64>,
    /// A growth experiment in flight: (connections before, rate before).
    pending_eval: Option<(usize, f64)>,
    samples_since_change: u32,
    grow_block_until: Instant,
    /// While set, a user-chosen connection count is temporarily overridden by a throttle.
    throttle_until: Option<Instant>,
    seen_throttle: u64,
    seen_resets: u64,
}

pub struct Controller {
    shared: Arc<RunShared>,
    join: JoinSet<WorkerOutcome>,
    adaptive: Adaptive,
    meter: SpeedMeter,
    meter_last_bytes: u64,
    meter_last_tick: Instant,
    peak_reported: u64,
}

impl Controller {
    pub fn new(shared: Arc<RunShared>, initial_target: usize) -> Self {
        let settings = shared.settings.clone();
        let enabled = shared
            .task
            .options
            .adaptive
            .unwrap_or(settings.network.adaptive_segmentation);
        let hard_max = (settings.network.max_connections_per_host as usize)
            .min(shared.limits.per_host_cap())
            .clamp(1, crate::hostlimits::ABSOLUTE_MAX_CONNECTIONS);
        let now = Instant::now();
        let downloaded = shared.control.counters.downloaded();
        Self {
            shared,
            join: JoinSet::new(),
            adaptive: Adaptive {
                enabled,
                target: initial_target.clamp(1, hard_max),
                hard_max,
                last_sample: now,
                last_bytes: downloaded,
                last_rate: 0.0,
                rate_at: BTreeMap::new(),
                pending_eval: None,
                samples_since_change: 0,
                grow_block_until: now,
                throttle_until: None,
                seen_throttle: 0,
                seen_resets: 0,
            },
            meter: SpeedMeter::new(),
            meter_last_bytes: downloaded,
            meter_last_tick: now,
            peak_reported: 0,
        }
    }

    pub async fn run(mut self) -> RunResult {
        let control = self.shared.control.clone();
        let mut progress_tick = tokio::time::interval(self.shared.tuning.progress_interval);
        let mut checkpoint_tick = tokio::time::interval(self.shared.tuning.checkpoint_interval);
        let mut adapt_tick = tokio::time::interval(self.shared.tuning.adapt_interval);
        progress_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        checkpoint_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        adapt_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // The first tick of an interval fires immediately; consume it.
        progress_tick.tick().await;
        checkpoint_tick.tick().await;
        adapt_tick.tick().await;

        loop {
            if let Some(r) = self.fill_slots().await {
                return self.finish(r).await;
            }
            if self.join.is_empty() && self.shared.plan.lock().is_complete() {
                // A final, exact progress line so the UI lands on 100 % from the engine.
                self.progress();
                return RunResult::Completed;
            }
            let wake = self.shared.plan.lock().next_wake();
            let wake_sleep = async move {
                match wake {
                    Some(t) => tokio::time::sleep_until(tokio::time::Instant::from_std(t)).await,
                    None => std::future::pending::<()>().await,
                }
            };
            let have_workers = !self.join.is_empty();
            tokio::select! {
                _ = control.stopped() => return self.stop().await,
                Some(res) = self.join.join_next(), if have_workers => {
                    let outcome = match res {
                        Ok(o) => o,
                        Err(e) => {
                            let err = TaskError::internal(format!("segment task panicked: {e}"));
                            return self.finish(RunResult::Failed(err)).await;
                        }
                    };
                    if let Some(r) = self.on_worker(outcome) {
                        return self.finish(r).await;
                    }
                }
                _ = progress_tick.tick() => {
                    if self.progress() {
                        return self.stop().await;
                    }
                }
                _ = checkpoint_tick.tick() => {
                    // One barrier flush per file per tick; `committed` moves only after it.
                    let shared = self.shared.clone();
                    let snapshot = shared.plan.lock().snapshot_written();
                    let flushed = tokio::select! {
                        _ = control.stopped() => return self.stop().await,
                        r = shared.writer.flush(FlushLevel::Barrier) => r,
                    };
                    match flushed {
                        Ok(()) => self.commit_and_emit(&snapshot),
                        Err(e) => return self.finish(RunResult::Failed(e)).await,
                    }
                }
                _ = adapt_tick.tick() => self.adapt(),
                _ = wake_sleep => {}
            }
        }
    }

    // ---------------------------------------------------------------------------------------
    // slots
    // ---------------------------------------------------------------------------------------

    /// Spawn workers until `running == target` or nothing can be started. Returns a terminal
    /// result only when something fatal happened (no mirror left, stop while waiting).
    async fn fill_slots(&mut self) -> Option<RunResult> {
        loop {
            let now = Instant::now();
            let (running, candidate) = {
                let mut plan = self.shared.plan.lock();
                let running = plan.running();
                if running >= self.adaptive.target {
                    return None;
                }
                let cand = match plan.pick_ready(now) {
                    Some(i) => Some(i),
                    None => plan
                        .splittable_running()
                        .and_then(|i| plan.split(i))
                        .inspect(|&n| {
                            let s = &plan.segments[n];
                            self.shared.stat(EngineStat::SegmentReassigned);
                            self.shared.log(
                                LogLevel::Debug,
                                "segment.split",
                                format!(
                                    "work-stealing: new segment {} covers {}-{}",
                                    s.index, s.start, s.end
                                ),
                            );
                        }),
                };
                (running, cand)
            };
            let seg = candidate?;
            // Connection caps: never block while something is running; block for the first
            // connection so a busy process still serves every task eventually.
            let permit = match self.shared.limits.try_acquire(&self.shared.host) {
                Some(p) => p,
                None if running > 0 => return None,
                None => {
                    let control = self.shared.control.clone();
                    tokio::select! {
                        _ = control.stopped() => return Some(RunResult::Stopped),
                        p = self.shared.limits.acquire(&self.shared.host) => match p {
                            Ok(p) => p,
                            Err(e) => return Some(RunResult::Failed(e)),
                        },
                    }
                }
            };
            let mirror = {
                let mut mirrors = self.shared.mirrors.lock();
                let plan = self.shared.plan.lock();
                let s = &plan.segments[seg];
                // A retried segment already had its mirror chosen when it failed.
                let keep =
                    s.attempts > 0 && mirrors.get(s.mirror).map(|m| m.healthy).unwrap_or(false);
                if keep {
                    Some(s.mirror)
                } else {
                    mirrors.pick(None)
                }
            };
            let Some(mirror) = mirror else {
                let err = self.shared.mirrors.lock().exhausted_error();
                return Some(RunResult::Failed(err));
            };
            let token = CancellationToken::new();
            self.shared
                .plan
                .lock()
                .mark_running(seg, mirror, token.clone());
            self.join
                .spawn(run_worker(self.shared.clone(), seg, mirror, token, permit));
        }
    }

    // ---------------------------------------------------------------------------------------
    // worker outcomes
    // ---------------------------------------------------------------------------------------

    fn on_worker(&mut self, o: WorkerOutcome) -> Option<RunResult> {
        match o.result {
            WorkerResult::Done => {
                self.shared.plan.lock().mark_done(o.seg);
                None
            }
            WorkerResult::Stopped => {
                self.shared.plan.lock().mark_pending(o.seg, None);
                None
            }
            WorkerResult::RangeUnsupported => Some(RunResult::RangeUnsupported),
            WorkerResult::Failed { error, origin } => {
                self.on_failure(o.seg, o.mirror, error, origin)
            }
        }
    }

    fn on_failure(
        &mut self,
        seg: usize,
        mirror: usize,
        error: TaskError,
        origin: FailureOrigin,
    ) -> Option<RunResult> {
        if origin == FailureOrigin::Disk {
            return Some(RunResult::Failed(error));
        }
        let class = error.class();
        let throttled = matches!(
            error.kind,
            ErrorKind::Throttled | ErrorKind::ServerError | ErrorKind::QuotaExceeded
        );
        let retry_after = error.retry_after_ms.map(Duration::from_millis);
        let multi = self.shared.mirrors.lock().is_multi();

        let attempts = {
            let mut plan = self.shared.plan.lock();
            let Some(s) = plan.segments.get_mut(seg) else {
                return Some(RunResult::Failed(TaskError::internal("segment vanished")));
            };
            if s.written > s.attempt_from {
                s.attempts = 0;
            }
            s.attempts += 1;
            s.last_error = Some(error.clone());
            s.attempts
        };

        // A throttle always costs a slot; the server said so.
        if throttled {
            self.on_throttle_signal(&error);
        }

        let mut next_mirror = mirror;
        let mut delay_class = class;
        if multi {
            let fatal = !class_can_retry(class);
            let (became_unhealthy, next) = {
                let mut mirrors = self.shared.mirrors.lock();
                let became = mirrors.record_failure(mirror, &error, fatal);
                (became, mirrors.pick(Some(mirror)))
            };
            if became_unhealthy {
                self.shared.log(
                    LogLevel::Warn,
                    "mirror.switch",
                    format!("mirror {mirror} disabled: {}", error.message),
                );
            }
            match next {
                Some(n) => {
                    if n != mirror {
                        self.shared
                            .stat(EngineStat::MirrorSwitched { index: n as u32 });
                        self.shared.log(
                            LogLevel::Info,
                            "mirror.switch",
                            format!(
                                "segment {seg}: mirror {mirror} -> {n} after {}",
                                error.message
                            ),
                        );
                    }
                    next_mirror = n;
                    // Another mirror is a fresh chance: do not let one bad host's permanent
                    // error end the task.
                    if !class_can_retry(class) {
                        delay_class = FailureClass::SourceProblem;
                    }
                }
                None => {
                    let err = self.shared.mirrors.lock().exhausted_error();
                    return Some(RunResult::Failed(err));
                }
            }
        } else if !class_can_retry(class) {
            return Some(RunResult::Failed(error));
        }

        let delay = self
            .shared
            .backoff
            .delay_for_with_hint(attempts, delay_class, retry_after);
        let Some(delay) = delay else {
            self.shared.log(
                LogLevel::Error,
                "segment.retry",
                format!(
                    "segment {seg}: giving up after {attempts} attempts: {}",
                    error.message
                ),
            );
            return Some(RunResult::Failed(error));
        };
        self.shared.stat(EngineStat::Retry);
        self.shared.log(
            LogLevel::Info,
            "segment.retry",
            format!(
                "segment {seg}: attempt {attempts} failed ({}); retrying in {:.1}s",
                error.message,
                delay.as_secs_f64()
            ),
        );
        let discarded = {
            let mut plan = self.shared.plan.lock();
            let d = if self.shared.ranges() {
                0
            } else {
                plan.reset_to_start(seg)
            };
            if let Some(s) = plan.segments.get_mut(seg) {
                s.mirror = next_mirror;
            }
            plan.mark_pending(seg, Some(Instant::now() + delay));
            d
        };
        if discarded > 0 {
            // Without ranges a retry restarts the body from byte 0.
            self.shared
                .stat(EngineStat::BytesDiscarded { bytes: discarded });
            let base = self
                .shared
                .control
                .counters
                .downloaded()
                .saturating_sub(discarded);
            self.shared.control.counters.set_downloaded(base);
            let writer = self.shared.writer.clone();
            tokio::spawn(async move {
                let _ = writer.truncate(0).await;
            });
        }
        None
    }

    fn on_throttle_signal(&mut self, error: &TaskError) {
        let running = self.shared.plan.lock().running();
        let new_target = running.max(1);
        if new_target < self.adaptive.target {
            self.shared.log(
                LogLevel::Info,
                "adaptive.shrink",
                format!(
                    "{} -> {} connections after {}",
                    self.adaptive.target, new_target, error.message
                ),
            );
            self.set_target(new_target);
        }
        let now = Instant::now();
        let cooldown = self.shared.tuning.throttle_cooldown;
        self.adaptive.grow_block_until = now + cooldown;
        self.adaptive.throttle_until = Some(now + cooldown);
        self.adaptive.pending_eval = None;
    }

    // ---------------------------------------------------------------------------------------
    // adaptive controller
    // ---------------------------------------------------------------------------------------

    fn set_target(&mut self, target: usize) {
        let target = target.clamp(1, self.adaptive.hard_max);
        if target == self.adaptive.target {
            return;
        }
        self.adaptive.target = target;
        self.adaptive.samples_since_change = 0;
        let running = self.shared.plan.lock().running();
        if running > target {
            self.cancel_workers(running - target);
        }
    }

    /// Stop `n` running workers, preferring those with the least left (cheapest to hand over).
    fn cancel_workers(&self, n: usize) {
        let plan = self.shared.plan.lock();
        let mut running: Vec<&crate::plan::SegState> = plan
            .segments
            .iter()
            .filter(|s| s.status == SegStatus::Running)
            .collect();
        running.sort_by_key(|s| s.remaining());
        for s in running.into_iter().take(n) {
            if let Some(t) = &s.stop {
                t.cancel();
            }
        }
    }

    fn adapt(&mut self) {
        let now = Instant::now();
        let dt = now
            .duration_since(self.adaptive.last_sample)
            .as_secs_f64()
            .max(0.05);
        let bytes = self.shared.control.counters.downloaded();
        let rate = bytes.saturating_sub(self.adaptive.last_bytes) as f64 / dt;
        self.adaptive.last_sample = now;
        self.adaptive.last_bytes = bytes;
        self.adaptive.samples_since_change += 1;
        let settled = self.adaptive.samples_since_change >= self.shared.tuning.settle_samples;

        // Per-segment windows feed mirror throughput and the slow-segment detector.
        let (windows, running, pending, total_known) = {
            let mut plan = self.shared.plan.lock();
            let w = plan.take_windows();
            let per_mirror: Vec<(usize, u64)> = w
                .iter()
                .map(|(i, b)| (plan.segments[*i].mirror, *b))
                .collect();
            drop(plan);
            let mut mirrors = self.shared.mirrors.lock();
            for (m, b) in per_mirror {
                mirrors.record_throughput(m, b, Duration::from_secs_f64(dt));
            }
            let plan = self.shared.plan.lock();
            (w, plan.running(), plan.pending(), plan.total.is_some())
        };

        if self.adaptive.last_rate > 64.0 * 1024.0 && rate < 0.5 * self.adaptive.last_rate {
            self.shared.stat(EngineStat::ThroughputDrop);
        }

        // --- explicit signals -------------------------------------------------------------
        let throttle = self.shared.throttle_events.load(Ordering::Relaxed);
        let resets = self.shared.reset_events.load(Ordering::Relaxed);
        let new_resets = resets.saturating_sub(self.adaptive.seen_resets);
        self.adaptive.seen_throttle = throttle;
        self.adaptive.seen_resets = resets;
        if new_resets >= 2 && self.adaptive.target > 1 {
            let t = self.adaptive.target - 1;
            self.shared.log(
                LogLevel::Info,
                "adaptive.shrink",
                format!(
                    "{new_resets} connection resets in {dt:.1}s; {} -> {t} connections",
                    self.adaptive.target
                ),
            );
            self.set_target(t);
            self.adaptive.grow_block_until = now + self.shared.tuning.grow_cooldown;
            self.adaptive.pending_eval = None;
        }

        // --- user override --------------------------------------------------------------
        let user_max = self.shared.control.max_connections() as usize;
        if user_max > 0 {
            let throttled = self
                .adaptive
                .throttle_until
                .map(|t| now < t)
                .unwrap_or(false);
            if !throttled {
                self.adaptive.throttle_until = None;
                self.set_target(user_max);
            } else if self.adaptive.target > user_max {
                self.set_target(user_max);
            }
        }

        // --- throttling detection: n connections no better than n/2 ---------------------
        if settled && running >= 3 && rate > 0.0 {
            let ceiling = running / 2;
            let flat = self
                .adaptive
                .rate_at
                .iter()
                .filter(|(k, _)| **k <= ceiling && **k >= 1)
                .find(|(_, r)| rate <= **r * 1.15)
                .map(|(k, _)| *k);
            if let Some(k) = flat {
                self.shared.stat(EngineStat::Throttled);
                self.shared.log(
                    LogLevel::Warn,
                    "throttle.detected",
                    format!(
                        "{running} connections deliver {:.0} KB/s, no more than {k} did; shrinking",
                        rate / 1024.0
                    ),
                );
                self.set_target(k);
                self.adaptive.grow_block_until = now + self.shared.tuning.throttle_cooldown;
                self.adaptive.pending_eval = None;
                self.adaptive.rate_at.retain(|n, _| *n <= k);
            }
        }

        if settled && running > 0 && rate > 0.0 {
            let e = self.adaptive.rate_at.entry(running).or_insert(rate);
            *e = 0.6 * *e + 0.4 * rate;
        }

        // --- judge a growth experiment ----------------------------------------------------
        if let Some((before_conns, before_rate)) = self.adaptive.pending_eval {
            if settled {
                let gain = (rate - before_rate) / before_rate.max(1.0);
                if gain >= self.shared.tuning.grow_gain_threshold {
                    self.shared.log(
                        LogLevel::Debug,
                        "adaptive.grow",
                        format!(
                            "{before_conns} -> {} connections: +{:.0}% throughput, kept",
                            self.adaptive.target,
                            gain * 100.0
                        ),
                    );
                } else {
                    let t = self.adaptive.target.saturating_sub(1).max(1);
                    self.shared.log(
                        LogLevel::Debug,
                        "adaptive.shrink",
                        format!(
                            "{} -> {t} connections: only {:+.0}% throughput from the extra connection",
                            self.adaptive.target,
                            gain * 100.0
                        ),
                    );
                    self.set_target(t);
                    self.adaptive.grow_block_until = now + self.shared.tuning.grow_cooldown;
                }
                self.adaptive.pending_eval = None;
            }
        } else if self.adaptive.enabled
            && user_max == 0
            && settled
            && now >= self.adaptive.grow_block_until
            && running >= self.adaptive.target
            && self.adaptive.target < self.adaptive.hard_max
            && rate > 0.0
            && total_known
            && self.shared.plan.lock().splittable_running().is_some()
        {
            self.adaptive.pending_eval = Some((running, rate));
            let t = self.adaptive.target + 1;
            self.shared.log(
                LogLevel::Debug,
                "adaptive.grow",
                format!("trying {} -> {t} connections", self.adaptive.target),
            );
            self.set_target(t);
        }

        // --- endgame: split a straggler ---------------------------------------------------
        if pending == 0 && running >= 2 && total_known && windows.len() >= 2 {
            let mut speeds: Vec<u64> = windows.iter().map(|(_, b)| *b).collect();
            speeds.sort_unstable();
            let median = speeds[speeds.len() / 2] as f64;
            if median > 0.0 {
                let slow = windows
                    .iter()
                    .filter(|(_, b)| (*b as f64) < median / self.shared.tuning.slow_segment_factor)
                    .min_by_key(|(_, b)| *b)
                    .map(|(i, _)| *i);
                if let Some(i) = slow {
                    let mut plan = self.shared.plan.lock();
                    if let Some(n) = plan.split(i) {
                        let s = &plan.segments[n];
                        self.shared.stat(EngineStat::SegmentReassigned);
                        self.shared.log(
                            LogLevel::Debug,
                            "segment.split",
                            format!(
                                "slow segment {} split; new segment {} covers {}-{}",
                                plan.segments[i].index, s.index, s.start, s.end
                            ),
                        );
                    }
                }
            }
        }

        self.adaptive.last_rate = rate;
    }

    // ---------------------------------------------------------------------------------------
    // progress / checkpoints / stop
    // ---------------------------------------------------------------------------------------

    /// Emit progress. Returns `true` when the run must pause (destination volume gone).
    fn progress(&mut self) -> bool {
        let now = Instant::now();
        let bytes = self.shared.control.counters.downloaded();
        let delta = bytes.saturating_sub(self.meter_last_bytes);
        self.meter_last_bytes = bytes;
        self.meter.record(delta);
        if now.duration_since(self.meter_last_tick) >= Duration::from_secs(1) {
            self.meter.tick();
            self.meter_last_tick = now;
        }
        let total = self.shared.expected_total;
        let remaining = total.map(|t| t.saturating_sub(bytes)).unwrap_or(0);
        let (active, fraction) = {
            let plan = self.shared.plan.lock();
            let done = plan
                .segments
                .iter()
                .filter(|s| s.status == SegStatus::Done)
                .count();
            let fraction = match total {
                Some(t) if t > 0 => (bytes as f64 / t as f64).min(1.0) as f32,
                _ => {
                    if plan.segments.is_empty() {
                        0.0
                    } else {
                        done as f32 / plan.segments.len() as f32
                    }
                }
            };
            (plan.running() as u32, fraction)
        };
        let speed = self.meter.smoothed();
        let instant = self.meter.instant();
        self.shared.sink.progress(Progress {
            downloaded: bytes,
            uploaded: 0,
            total,
            speed,
            instant_speed: instant,
            upload_speed: 0,
            eta_seconds: if total.is_some() {
                self.meter.eta(remaining)
            } else {
                None
            },
            active_connections: active,
            peers: 0,
            seeds: 0,
            ratio: 0.0,
            fraction,
        });
        if self.meter.peak() > self.peak_reported {
            self.peak_reported = self.meter.peak();
            self.shared.stat(EngineStat::Peak {
                speed: self.peak_reported,
            });
        }
        self.shared.control.volume_lost.load(Ordering::Relaxed)
    }

    fn commit_and_emit(&self, snapshot: &[u64]) {
        let map = {
            let mut plan = self.shared.plan.lock();
            plan.commit(snapshot);
            plan.to_map(
                self.shared.etag.clone(),
                self.shared.last_modified.clone(),
                Some(self.shared.part_path.clone()),
            )
        };
        if self.shared.resumable() {
            self.shared.sink.checkpoint(Checkpoint::Segments(map));
        }
    }

    /// Pause/cancel protocol: stop reads, bounded flush, checkpoint only on success.
    async fn stop(mut self) -> RunResult {
        self.cancel_all().await;
        let snapshot = self.shared.plan.lock().snapshot_written();
        let flush = tokio::time::timeout(
            self.shared.tuning.stop_flush_timeout,
            self.shared.writer.flush(FlushLevel::Barrier),
        )
        .await;
        match flush {
            Ok(Ok(())) => self.commit_and_emit(&snapshot),
            Ok(Err(e)) => self.shared.log(
                LogLevel::Warn,
                "http.pause",
                format!(
                    "flush failed on pause; keeping the previous checkpoint: {}",
                    e.message
                ),
            ),
            Err(_) => self.shared.log(
                LogLevel::Warn,
                "http.pause",
                "flush timed out on pause; keeping the previous checkpoint",
            ),
        }
        RunResult::Stopped
    }

    async fn finish(mut self, result: RunResult) -> RunResult {
        self.cancel_all().await;
        result
    }

    async fn cancel_all(&mut self) {
        {
            let plan = self.shared.plan.lock();
            for s in &plan.segments {
                if let Some(t) = &s.stop {
                    t.cancel();
                }
            }
        }
        // Workers select on their token at every await; give them a moment, then abort.
        let drain = async {
            while let Some(res) = self.join.join_next().await {
                if let Ok(o) = res {
                    let mut plan = self.shared.plan.lock();
                    match o.result {
                        WorkerResult::Done => plan.mark_done(o.seg),
                        _ => plan.mark_pending(o.seg, None),
                    }
                }
            }
        };
        if tokio::time::timeout(Duration::from_secs(3), drain)
            .await
            .is_err()
        {
            self.join.abort_all();
            while self.join.join_next().await.is_some() {}
            let mut plan = self.shared.plan.lock();
            for i in 0..plan.segments.len() {
                if plan.segments[i].status == SegStatus::Running {
                    plan.mark_pending(i, None);
                }
            }
        }
    }
}

fn class_can_retry(class: FailureClass) -> bool {
    matches!(
        class,
        FailureClass::Transient
            | FailureClass::Throttled
            | FailureClass::SourceProblem
            | FailureClass::Degrade
    )
}
