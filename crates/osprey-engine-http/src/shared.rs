//! State shared by the controller and every segment worker for the duration of one run.

use crate::hostlimits::HostLimits;
use crate::mirrors::MirrorSet;
use crate::plan::Plan;
use crate::request::RequestTemplate;
use osprey_domain::events::LogLevel;
use osprey_domain::settings::Settings;
use osprey_domain::Task;
use osprey_runtime::backoff::BackoffPolicy;
use osprey_runtime::disk::FileWriter;
use osprey_runtime::engine::{EngineStat, ProgressSink, TransferControl};
use osprey_runtime::redact::redact;
use osprey_runtime::RateLimiter;
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Timing and threshold knobs. Production uses [`Tuning::default`]; tests shorten the
/// intervals so adaptive behaviour is observable on a local server.
#[derive(Clone, Debug)]
pub struct Tuning {
    /// `sink.progress` cadence (≤ 4/s).
    pub progress_interval: Duration,
    /// Barrier flush + checkpoint cadence.
    pub checkpoint_interval: Duration,
    /// Adaptive controller sampling period.
    pub adapt_interval: Duration,
    /// Per-mirror probe timeout.
    pub probe_timeout: Duration,
    /// Bound on the flush performed when pausing/cancelling.
    pub stop_flush_timeout: Duration,
    /// Minimum relative throughput gain for a grown connection to be kept.
    pub grow_gain_threshold: f64,
    /// A segment slower than `median / slow_segment_factor` is split near the end.
    pub slow_segment_factor: f64,
    /// Samples to wait after a concurrency change before judging it.
    pub settle_samples: u32,
    /// Grow cooldown after a rejected growth or a throttle signal.
    pub grow_cooldown: Duration,
    pub throttle_cooldown: Duration,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            progress_interval: Duration::from_millis(250),
            checkpoint_interval: Duration::from_secs(3),
            adapt_interval: Duration::from_secs(2),
            probe_timeout: Duration::from_secs(5),
            stop_flush_timeout: Duration::from_secs(5),
            grow_gain_threshold: 0.15,
            slow_segment_factor: 4.0,
            settle_samples: 2,
            grow_cooldown: Duration::from_secs(20),
            throttle_cooldown: Duration::from_secs(30),
        }
    }
}

pub struct RunShared {
    pub task: Task,
    pub settings: Arc<Settings>,
    pub control: Arc<TransferControl>,
    pub sink: Arc<dyn ProgressSink>,
    pub limiter: Arc<RateLimiter>,
    pub client: reqwest::Client,
    pub template: RequestTemplate,
    pub writer: FileWriter,
    pub plan: Mutex<Plan>,
    pub mirrors: Mutex<MirrorSet>,
    pub limits: Arc<HostLimits>,
    /// Host key for the connection caps (primary URL's host).
    pub host: String,
    pub tuning: Tuning,
    pub backoff: BackoffPolicy,
    /// `true` while segments are fetched with `Range`; cleared when the server ignores ranges.
    pub ranges: AtomicBool,
    pub expected_total: Option<u64>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    /// Compare ETag / Last-Modified of every response against the probe (single source only;
    /// mirrors legitimately differ).
    pub validate_validators: bool,
    /// A binary was expected: an HTML body is an interstitial, not the file.
    pub expect_binary: bool,
    pub part_path: PathBuf,
    /// Adaptive-controller signals bumped by workers.
    pub throttle_events: AtomicU64,
    pub reset_events: AtomicU64,
}

impl RunShared {
    pub fn ranges(&self) -> bool {
        self.ranges.load(Ordering::Relaxed)
    }

    pub fn log(&self, level: LogLevel, code: &str, message: impl AsRef<str>) {
        self.sink.log(level, code, redact(message.as_ref()));
    }

    pub fn stat(&self, stat: EngineStat) {
        self.sink.stat(stat);
    }

    /// Whether a checkpoint can ever be useful (ranges honoured and size known).
    pub fn resumable(&self) -> bool {
        self.ranges() && self.expected_total.is_some()
    }
}
