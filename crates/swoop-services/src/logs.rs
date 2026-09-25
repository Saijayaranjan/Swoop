//! Process logging: a `tracing` subscriber with a redacted 2,000-line ring buffer (served by
//! `recent_logs`) and a daily rolling file in the log directory. Initialised once per process.

use parking_lot::Mutex;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use swoop_runtime::redact::redact;
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{reload, EnvFilter, Registry};

/// Lines kept in memory.
pub const RING_CAPACITY: usize = 2_000;

/// One captured log line.
#[derive(Clone, Debug)]
pub struct LogLine {
    pub level: tracing::Level,
    pub text: String,
}

/// The in-memory ring.
#[derive(Default)]
pub struct LogRing {
    lines: Mutex<VecDeque<LogLine>>,
}

impl LogRing {
    fn push(&self, line: LogLine) {
        let mut l = self.lines.lock();
        if l.len() >= RING_CAPACITY {
            l.pop_front();
        }
        l.push_back(line);
    }

    /// Newest `limit` lines (oldest first), optionally at or above `level`.
    pub fn recent(&self, limit: usize, level: Option<tracing::Level>) -> Vec<String> {
        let l = self.lines.lock();
        let filtered: Vec<&LogLine> = l
            .iter()
            .filter(|x| level.map(|lv| x.level <= lv).unwrap_or(true))
            .collect();
        let skip = if limit == 0 {
            0
        } else {
            filtered.len().saturating_sub(limit)
        };
        filtered
            .into_iter()
            .skip(skip)
            .map(|x| x.text.clone())
            .collect()
    }
}

struct RingLayer {
    ring: Arc<LogRing>,
}

#[derive(Default)]
struct MessageVisitor {
    message: String,
    fields: Vec<(String, String)>,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            self.fields
                .push((field.name().to_owned(), format!("{value:?}")));
        }
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_owned();
        } else {
            self.fields
                .push((field.name().to_owned(), value.to_owned()));
        }
    }
}

impl<S: tracing::Subscriber> Layer<S> for RingLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut v = MessageVisitor::default();
        event.record(&mut v);
        let meta = event.metadata();
        let mut text = format!(
            "{} {:<5} {}: {}",
            chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"),
            meta.level(),
            meta.target(),
            v.message
        );
        for (k, val) in v.fields {
            text.push(' ');
            text.push_str(&k);
            text.push('=');
            text.push_str(&val);
        }
        self.ring.push(LogLine {
            level: *meta.level(),
            text: redact(&text),
        });
    }
}

struct Global {
    ring: Arc<LogRing>,
    reload: Option<reload::Handle<EnvFilter, Registry>>,
    _guard: Option<tracing_appender::non_blocking::WorkerGuard>,
}

static GLOBAL: OnceLock<Global> = OnceLock::new();

/// Handle returned by [`init_logging`]; keeps the ring reachable.
#[derive(Clone)]
pub struct LogHandle {
    ring: Arc<LogRing>,
}

impl LogHandle {
    pub fn ring(&self) -> Arc<LogRing> {
        self.ring.clone()
    }
}

fn filter_for(level: &str) -> EnvFilter {
    let level = match level.to_ascii_lowercase().as_str() {
        "trace" | "debug" | "info" | "warn" | "error" => level.to_ascii_lowercase(),
        _ => "info".to_owned(),
    };
    EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(format!(
            "{level},hyper=warn,reqwest=warn,rustls=warn,librqbit=info"
        ))
    })
}

/// Install the subscriber (idempotent: later calls return the existing ring). `log_dir`
/// receives `swoop.log.<date>` files; files older than `retention_days` are removed.
pub fn init_logging(log_dir: &Path, level: &str, retention_days: u32) -> LogHandle {
    if let Some(g) = GLOBAL.get() {
        return LogHandle {
            ring: g.ring.clone(),
        };
    }
    let ring = Arc::new(LogRing::default());
    prune_old_logs(log_dir, retention_days);
    let (filter, handle) = reload::Layer::new(filter_for(level));
    let (file_layer, guard) = match std::fs::create_dir_all(log_dir) {
        Ok(()) => {
            let appender = tracing_appender::rolling::daily(log_dir, "swoop.log");
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let layer = tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_target(true)
                .with_writer(writer);
            (Some(layer), Some(guard))
        }
        Err(_) => (None, None),
    };
    let subscriber = tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(RingLayer { ring: ring.clone() });
    let installed = subscriber.try_init().is_ok();
    let global = Global {
        ring: ring.clone(),
        reload: installed.then_some(handle),
        _guard: guard,
    };
    let _ = GLOBAL.set(global);
    if !installed {
        tracing::debug!("a tracing subscriber was already installed; ring buffer inactive");
    }
    LogHandle {
        ring: GLOBAL.get().map(|g| g.ring.clone()).unwrap_or(ring),
    }
}

/// Change the level filter at runtime (settings change).
pub fn set_level(level: &str) {
    if let Some(h) = GLOBAL.get().and_then(|g| g.reload.as_ref()) {
        if let Err(e) = h.reload(filter_for(level)) {
            tracing::debug!(error = %e, "log level not changed");
        }
    }
}

/// The global ring, if logging was initialised.
pub fn ring() -> Option<Arc<LogRing>> {
    GLOBAL.get().map(|g| g.ring.clone())
}

/// Parse a level name for `recent_logs`.
pub fn parse_level(s: &str) -> Option<tracing::Level> {
    match s.to_ascii_lowercase().as_str() {
        "trace" => Some(tracing::Level::TRACE),
        "debug" => Some(tracing::Level::DEBUG),
        "info" => Some(tracing::Level::INFO),
        "warn" | "warning" => Some(tracing::Level::WARN),
        "error" => Some(tracing::Level::ERROR),
        _ => None,
    }
}

fn prune_old_logs(log_dir: &Path, retention_days: u32) {
    if retention_days == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return;
    };
    let cutoff = std::time::SystemTime::now()
        - std::time::Duration::from_secs(u64::from(retention_days) * 86_400);
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.starts_with("swoop.log") {
            continue;
        }
        if let Ok(meta) = e.metadata() {
            if meta.modified().map(|m| m < cutoff).unwrap_or(false) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}
