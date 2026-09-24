//! Shared router state, the authenticated caller and the per-token rate limiter.

use crate::ListenerKind;
use dashmap::DashMap;
use osprey_domain::device::{Device, Scope};
use osprey_services::SharedEngine;
use std::sync::Arc;
use std::time::Instant;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub(crate) struct RouterConfig {
    pub kind: ListenerKind,
    pub allowed_origins: Vec<String>,
    /// 0 = unlimited.
    pub rate_limit_per_minute: u32,
    pub tls_fingerprint: Option<String>,
    pub serve_web_ui: bool,
}

#[derive(Clone)]
pub(crate) struct AppState(Arc<Inner>);

pub(crate) struct Inner {
    pub engine: SharedEngine,
    pub cfg: RouterConfig,
    pub shutdown: CancellationToken,
    pub limiter: RateLimiter,
}

impl std::ops::Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl AppState {
    pub fn new(engine: SharedEngine, cfg: RouterConfig, shutdown: CancellationToken) -> Self {
        let limiter = RateLimiter::new(cfg.rate_limit_per_minute);
        Self(Arc::new(Inner {
            engine,
            cfg,
            shutdown,
            limiter,
        }))
    }
}

/// The client address as a string (`unix` for the Unix socket, `unknown` without connect info).
#[derive(Clone, Debug)]
pub(crate) struct ClientIp(pub String);

/// The authenticated caller, inserted into request extensions by the auth middleware.
#[derive(Clone, Debug)]
pub(crate) struct Caller {
    pub device: Device,
    /// The local token on a local listener: the machine's own user (CLI, native host, app).
    /// Everything else — any paired device on any listener — is treated as remote.
    pub trusted: bool,
}

impl Caller {
    pub fn has(&self, scope: Scope) -> bool {
        self.trusted || self.device.has_scope(scope)
    }
}

/// Token bucket per key (device id, or `ip:<addr>` for unauthenticated pairing requests).
pub(crate) struct RateLimiter {
    per_minute: u32,
    buckets: DashMap<String, Bucket>,
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

const MAX_BUCKETS: usize = 10_000;

impl RateLimiter {
    pub fn new(per_minute: u32) -> Self {
        Self {
            per_minute,
            buckets: DashMap::new(),
        }
    }

    /// `Ok(())` if the request may proceed, `Err(seconds)` to wait otherwise.
    pub fn check(&self, key: &str) -> Result<(), u64> {
        if self.per_minute == 0 {
            return Ok(());
        }
        let capacity = f64::from(self.per_minute);
        let rate = capacity / 60.0;
        let now = Instant::now();
        if self.buckets.len() > MAX_BUCKETS {
            // Drop buckets that have fully refilled; they carry no state worth keeping.
            let full_after = 60.0;
            self.buckets
                .retain(|_, b| now.duration_since(b.last).as_secs_f64() < full_after);
        }
        let mut b = self.buckets.entry(key.to_owned()).or_insert(Bucket {
            tokens: capacity,
            last: now,
        });
        let elapsed = now.duration_since(b.last).as_secs_f64();
        b.tokens = (b.tokens + elapsed * rate).min(capacity);
        b.last = now;
        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            Ok(())
        } else {
            let wait = ((1.0 - b.tokens) / rate).ceil();
            Err(wait.max(1.0) as u64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_limits_and_refills() {
        let l = RateLimiter::new(3);
        assert!(l.check("a").is_ok());
        assert!(l.check("a").is_ok());
        assert!(l.check("a").is_ok());
        let wait = l.check("a").unwrap_err();
        assert!((1..=20).contains(&wait));
        assert!(l.check("b").is_ok());
        assert!(RateLimiter::new(0).check("x").is_ok());
    }
}
