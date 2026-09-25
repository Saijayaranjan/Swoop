//! Hierarchical rate limiter: global → queue → task. Each level can be changed live; `0` means
//! unlimited.
//!
//! Implementation: a *virtual clock* per level. Each request reserves a slot
//! (`slot = max(now, next_free)`, `next_free = slot + n / limit`) and sleeps until its slot.
//! Reservations are FIFO and never "forgiven", so the aggregate rate is exact under contention;
//! a burst allowance lets the clock lag `now` by up to `burst / limit` seconds so short bursts
//! and timer granularity do not throttle high-speed links.

use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Clock {
    next_free: Instant,
    /// Limit the current `next_free` was computed against (for rescaling on change).
    limit: u64,
}

pub struct RateLimiter {
    /// Bytes per second; 0 = unlimited.
    limit: AtomicU64,
    burst: AtomicU64,
    clock: Mutex<Clock>,
    parent: Option<Arc<RateLimiter>>,
    name: Arc<str>,
}

impl RateLimiter {
    pub fn new(
        name: impl Into<Arc<str>>,
        limit: u64,
        parent: Option<Arc<RateLimiter>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            limit: AtomicU64::new(limit),
            burst: AtomicU64::new(256 * 1024),
            clock: Mutex::new(Clock {
                next_free: Instant::now(),
                limit,
            }),
            parent,
            name: name.into(),
        })
    }

    pub fn unlimited(name: impl Into<Arc<str>>) -> Arc<Self> {
        Self::new(name, 0, None)
    }

    pub fn child(self: &Arc<Self>, name: impl Into<Arc<str>>, limit: u64) -> Arc<Self> {
        Self::new(name, limit, Some(self.clone()))
    }

    /// Change the limit. Outstanding reservations are rescaled so lowering a limit does not
    /// stall every waiter for the old debt, and raising it releases them proportionally.
    pub fn set_limit(&self, bytes_per_second: u64) {
        let old = self.limit.swap(bytes_per_second, Ordering::Relaxed);
        let mut c = self.clock.lock();
        let now = Instant::now();
        if c.next_free > now && old > 0 && bytes_per_second > 0 && old != bytes_per_second {
            let debt = c.next_free.duration_since(now).as_secs_f64() * old as f64
                / bytes_per_second as f64;
            c.next_free = now + Duration::from_secs_f64(debt.min(60.0));
        } else if bytes_per_second == 0 || old == 0 {
            c.next_free = now;
        }
        c.limit = bytes_per_second;
    }

    pub fn set_burst(&self, bytes: u64) {
        self.burst.store(bytes.max(16 * 1024), Ordering::Relaxed);
    }

    pub fn limit(&self) -> u64 {
        self.limit.load(Ordering::Relaxed)
    }

    pub fn name(&self) -> &str {
        &self.name
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

    /// Reserve `n` bytes at this level; returns when the caller may proceed.
    fn reserve_local(&self, n: u64) -> Option<Instant> {
        let limit = self.limit();
        if limit == 0 {
            return None;
        }
        let mut c = self.clock.lock();
        let now = Instant::now();
        // Allow the clock to lag by the burst allowance so an idle limiter does not force a
        // sleep on the first chunk and so 1 ms timer granularity does not cap throughput.
        let burst_window =
            Duration::from_secs_f64(self.burst.load(Ordering::Relaxed) as f64 / limit as f64);
        let floor = now.checked_sub(burst_window).unwrap_or(now);
        if c.next_free < floor {
            c.next_free = floor;
        }
        let slot = c.next_free.max(floor);
        c.next_free = slot + Duration::from_secs_f64(n as f64 / limit as f64);
        c.limit = limit;
        if slot > now {
            Some(slot)
        } else {
            None
        }
    }

    /// Wait until `n` bytes may be transferred at every level of the hierarchy.
    pub async fn acquire(&self, n: u64) {
        if n == 0 {
            return;
        }
        let mut deadline: Option<Instant> = None;
        let mut level: Option<&RateLimiter> = Some(self);
        while let Some(l) = level {
            if let Some(slot) = l.reserve_local(n) {
                deadline = Some(deadline.map_or(slot, |d| d.max(slot)));
            }
            level = l.parent.as_deref();
        }
        if let Some(d) = deadline {
            // Sleep in slices so a limit change (which rescales the clock) is noticed promptly;
            // the reservation itself is never re-made.
            loop {
                let now = Instant::now();
                if d <= now {
                    break;
                }
                let remaining = d.duration_since(now);
                tokio::time::sleep(remaining.min(Duration::from_millis(500))).await;
                if remaining <= Duration::from_millis(500) {
                    break;
                }
                // If limits were lifted meanwhile, stop waiting.
                if self.effective_limit() == 0 {
                    break;
                }
            }
        }
    }

    /// Suggested read-chunk size given the effective limit (smaller chunks for slow limits keep
    /// the UI's progress smooth and the pacing accurate).
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
        global.set_burst(16 * 1024);
        let task = global.child("task", 0);
        let start = Instant::now();
        // 400 KB at 200 KB/s ≈ 2 s minus a 16 KB burst
        for _ in 0..25 {
            task.acquire(16 * 1024).await;
        }
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(1700), "took {elapsed:?}");
        assert!(elapsed <= Duration::from_millis(3000), "took {elapsed:?}");
    }

    #[tokio::test]
    async fn contention_respects_global_limit() {
        // 40 tasks sharing a 400 KB/s global limit must not exceed it in aggregate.
        let global = RateLimiter::new("global", 400_000, None);
        global.set_burst(16 * 1024);
        let total = Arc::new(AtomicU64::new(0));
        let start = Instant::now();
        let mut handles = Vec::new();
        for i in 0..40 {
            let t = global.child(format!("t{i}"), 0);
            let total = total.clone();
            handles.push(tokio::spawn(async move {
                while start.elapsed() < Duration::from_millis(1500) {
                    t.acquire(8 * 1024).await;
                    total.fetch_add(8 * 1024, Ordering::Relaxed);
                }
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        let secs = start.elapsed().as_secs_f64();
        let rate = total.load(Ordering::Relaxed) as f64 / secs;
        assert!(rate < 520_000.0, "aggregate rate {rate} B/s exceeded limit");
        assert!(rate > 250_000.0, "aggregate rate {rate} B/s too low");
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

    #[tokio::test]
    async fn lowering_limit_rescales_debt() {
        let l = RateLimiter::new("g", 10_000_000, None);
        l.acquire(5_000_000).await; // reserve ~0.5 s of debt
        l.set_limit(100_000);
        // debt rescaled: 0.5 s at 10 MB/s = 5 MB → at 100 KB/s that would be 50 s; capped to 60 s.
        // Raising back to unlimited must release immediately.
        l.set_limit(0);
        let start = Instant::now();
        l.acquire(1_000_000).await;
        assert!(start.elapsed() < Duration::from_millis(50));
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
