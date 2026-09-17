//! Hierarchical token-bucket rate limiter: global → queue → task. Each level can be changed
//! live; `0` means unlimited. Consumers call [`RateLimiter::acquire`] before writing a chunk
//! and it waits (fairly, FIFO per level) until tokens are available at every level.

use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Bucket {
    tokens: f64,
    last: Instant,
}

pub struct RateLimiter {
    /// Bytes per second; 0 = unlimited.
    limit: AtomicU64,
    burst: AtomicU64,
    bucket: Mutex<Bucket>,
    parent: Option<Arc<RateLimiter>>,
    name: &'static str,
}

impl RateLimiter {
    pub fn new(name: &'static str, limit: u64, parent: Option<Arc<RateLimiter>>) -> Arc<Self> {
        Arc::new(Self {
            limit: AtomicU64::new(limit),
            burst: AtomicU64::new(256 * 1024),
            bucket: Mutex::new(Bucket { tokens: 0.0, last: Instant::now() }),
            parent,
            name,
        })
    }

    pub fn unlimited(name: &'static str) -> Arc<Self> {
        Self::new(name, 0, None)
    }

    pub fn child(self: &Arc<Self>, name: &'static str, limit: u64) -> Arc<Self> {
        Self::new(name, limit, Some(self.clone()))
    }

    pub fn set_limit(&self, bytes_per_second: u64) {
        self.limit.store(bytes_per_second, Ordering::Relaxed);
        let mut b = self.bucket.lock();
        b.tokens = b.tokens.min(self.capacity());
    }

    pub fn set_burst(&self, bytes: u64) {
        self.burst.store(bytes.max(16 * 1024), Ordering::Relaxed);
    }

    pub fn limit(&self) -> u64 {
        self.limit.load(Ordering::Relaxed)
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Effective limit along the chain (minimum non-zero).
    pub fn effective_limit(&self) -> u64 {
        let mine = self.limit();
        match &self.parent {
            Some(p) => {
                let theirs = p.effective_limit();
                match (mine, theirs) {
                    (0, t) => t,
                    (m, 0) => m,
                    (m, t) => m.min(t),
                }
            }
            None => mine,
        }
    }

    fn capacity(&self) -> f64 {
        let limit = self.limit() as f64;
        let burst = self.burst.load(Ordering::Relaxed) as f64;
        // allow one second of traffic or the burst size, whichever is larger, to keep
        // high-speed links saturated without micro-sleeps.
        limit.max(burst)
    }

    /// Try to take `n` tokens at this level only. Returns the wait needed if not available.
    fn try_take_local(&self, n: u64) -> Option<Duration> {
        let limit = self.limit();
        if limit == 0 {
            return None;
        }
        let mut b = self.bucket.lock();
        let now = Instant::now();
        let elapsed = now.duration_since(b.last).as_secs_f64();
        b.last = now;
        b.tokens = (b.tokens + elapsed * limit as f64).min(self.capacity());
        let n = n as f64;
        if b.tokens >= n {
            b.tokens -= n;
            None
        } else {
            let deficit = n - b.tokens;
            // take everything now, go negative, and tell the caller how long to sleep so the
            // debt is repaid; this keeps ordering fair and avoids thundering herds.
            b.tokens -= n;
            Some(Duration::from_secs_f64(deficit / limit as f64))
        }
    }

    /// Wait until `n` bytes may be transferred at every level of the hierarchy.
    pub async fn acquire(&self, n: u64) {
        if n == 0 {
            return;
        }
        // chunk large requests so a single 4 MiB write cannot starve siblings
        let step = 64 * 1024;
        let mut remaining = n;
        while remaining > 0 {
            let take = remaining.min(step);
            let mut wait = Duration::ZERO;
            let mut level: Option<&RateLimiter> = Some(self);
            while let Some(l) = level {
                if let Some(w) = l.try_take_local(take) {
                    wait = wait.max(w);
                }
                level = l.parent.as_deref();
            }
            if wait > Duration::ZERO {
                tokio::time::sleep(wait.min(Duration::from_secs(2))).await;
            }
            remaining -= take;
        }
    }

    /// Suggested read-chunk size given the effective limit (smaller chunks for slow limits keep
    /// the UI's progress smooth).
    pub fn suggested_chunk(&self) -> usize {
        match self.effective_limit() {
            0 => 256 * 1024,
            l if l < 64 * 1024 => 4 * 1024,
            l if l < 1024 * 1024 => 16 * 1024,
            l if l < 16 * 1024 * 1024 => 64 * 1024,
            _ => 256 * 1024,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn limits_throughput() {
        let global = RateLimiter::new("global", 200_000, None); // 200 KB/s
        let task = global.child("task", 0);
        task.set_burst(16 * 1024);
        global.set_burst(16 * 1024);
        let start = Instant::now();
        // 400 KB should take ~2 s minus burst
        for _ in 0..25 {
            task.acquire(16 * 1024).await;
        }
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(1500), "took {elapsed:?}");
        assert!(elapsed <= Duration::from_millis(4000), "took {elapsed:?}");
    }

    #[tokio::test]
    async fn unlimited_is_instant() {
        let l = RateLimiter::unlimited("g");
        let start = Instant::now();
        for _ in 0..1000 {
            l.acquire(1024 * 1024).await;
        }
        assert!(start.elapsed() < Duration::from_millis(200));
    }

    #[test]
    fn effective_limit_is_min_nonzero() {
        let g = RateLimiter::new("g", 100, None);
        let q = g.child("q", 0);
        let t = q.child("t", 50);
        assert_eq!(t.effective_limit(), 50);
        g.set_limit(20);
        assert_eq!(t.effective_limit(), 20);
        g.set_limit(0);
        q.set_limit(0);
        t.set_limit(0);
        assert_eq!(t.effective_limit(), 0);
    }
}
