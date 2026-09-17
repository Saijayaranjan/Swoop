//! Speed measurement: instantaneous and smoothed throughput, ETA, peak tracking.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct SpeedMeter {
    window: Duration,
    samples: VecDeque<(Instant, u64)>,
    total: u64,
    smoothed: f64,
    last_tick: Instant,
    peak: u64,
    /// Recent smoothed samples for coefficient-of-variation calculation.
    history: VecDeque<u64>,
    drops: u32,
}

impl SpeedMeter {
    pub fn new() -> Self {
        Self::with_window(Duration::from_secs(3))
    }

    pub fn with_window(window: Duration) -> Self {
        let now = Instant::now();
        Self {
            window,
            samples: VecDeque::new(),
            total: 0,
            smoothed: 0.0,
            last_tick: now,
            peak: 0,
            history: VecDeque::new(),
            drops: 0,
        }
    }

    /// Record `bytes` transferred now.
    pub fn record(&mut self, bytes: u64) {
        let now = Instant::now();
        self.total += bytes;
        if let Some(back) = self.samples.back_mut() {
            if now.duration_since(back.0) < Duration::from_millis(100) {
                back.1 += bytes;
                return;
            }
        }
        self.samples.push_back((now, bytes));
        self.prune(now);
    }

    fn prune(&mut self, now: Instant) {
        while let Some(front) = self.samples.front() {
            if now.duration_since(front.0) > self.window {
                self.samples.pop_front();
            } else {
                break;
            }
        }
    }

    /// Bytes per second over the window.
    pub fn instant(&mut self) -> u64 {
        let now = Instant::now();
        self.prune(now);
        let Some(front) = self.samples.front() else {
            return 0;
        };
        let span = now.duration_since(front.0).as_secs_f64().max(0.25);
        let sum: u64 = self.samples.iter().map(|s| s.1).sum();
        (sum as f64 / span) as u64
    }

    /// Call ~once per second: updates smoothed speed, peak, drop counter.
    pub fn tick(&mut self) -> u64 {
        let inst = self.instant() as f64;
        let now = Instant::now();
        let dt = now
            .duration_since(self.last_tick)
            .as_secs_f64()
            .clamp(0.05, 5.0);
        self.last_tick = now;
        // time-corrected EMA with ~2 s constant
        let alpha = 1.0 - (-dt / 2.0).exp();
        let prev = self.smoothed;
        self.smoothed += alpha * (inst - self.smoothed);
        let s = self.smoothed as u64;
        if s > self.peak {
            self.peak = s;
        }
        if prev > 0.0 && self.smoothed < prev * 0.5 && prev > 64.0 * 1024.0 {
            self.drops += 1;
        }
        self.history.push_back(s);
        if self.history.len() > 60 {
            self.history.pop_front();
        }
        s
    }

    pub fn smoothed(&self) -> u64 {
        self.smoothed as u64
    }
    pub fn peak(&self) -> u64 {
        self.peak
    }
    pub fn total(&self) -> u64 {
        self.total
    }
    pub fn drops(&self) -> u32 {
        self.drops
    }

    /// Coefficient of variation of the recent smoothed samples (0 = perfectly steady).
    pub fn coefficient_of_variation(&self) -> f32 {
        if self.history.len() < 5 {
            return 0.0;
        }
        let n = self.history.len() as f64;
        let mean = self.history.iter().map(|&v| v as f64).sum::<f64>() / n;
        if mean <= 0.0 {
            return 0.0;
        }
        let var = self
            .history
            .iter()
            .map(|&v| (v as f64 - mean).powi(2))
            .sum::<f64>()
            / n;
        (var.sqrt() / mean) as f32
    }

    pub fn eta(&self, remaining: u64) -> Option<u64> {
        let s = self.smoothed;
        if s < 1.0 {
            return None;
        }
        Some((remaining as f64 / s).ceil() as u64)
    }
}

impl Default for SpeedMeter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_throughput() {
        let mut m = SpeedMeter::new();
        m.record(1000);
        std::thread::sleep(Duration::from_millis(120));
        m.record(1000);
        let inst = m.instant();
        assert!(inst > 0);
        assert_eq!(m.total(), 2000);
        let s = m.tick();
        assert!(s > 0);
        assert!(m.eta(10_000).is_some());
    }
}
