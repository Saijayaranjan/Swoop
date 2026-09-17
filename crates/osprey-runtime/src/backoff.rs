//! Exponential backoff with full jitter, bounded, and per-failure-class multipliers.

use osprey_domain::FailureClass;
use rand::Rng;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct BackoffPolicy {
    pub base: Duration,
    pub max: Duration,
    pub max_retries: u32,
}

impl Default for BackoffPolicy {
    fn default() -> Self {
        Self { base: Duration::from_secs(1), max: Duration::from_secs(60), max_retries: 8 }
    }
}

impl BackoffPolicy {
    pub fn from_settings(n: &osprey_domain::settings::NetworkSettings) -> Self {
        Self {
            base: Duration::from_millis(n.retry_base_delay_ms.max(100)),
            max: Duration::from_millis(n.retry_max_delay_ms.max(n.retry_base_delay_ms)),
            max_retries: n.max_retries,
        }
    }

    /// Delay before attempt number `attempt` (1-based). Returns `None` when retries are exhausted.
    pub fn delay_for(&self, attempt: u32, class: FailureClass) -> Option<Duration> {
        if attempt == 0 || attempt > self.max_retries {
            return None;
        }
        let multiplier = match class {
            FailureClass::Transient => 1.0,
            FailureClass::Throttled => 3.0,
            FailureClass::SourceProblem => 2.0,
            FailureClass::NeedsUser | FailureClass::Permanent => return None,
            FailureClass::RestartFromScratch => 1.0,
        };
        let exp = self.base.as_millis() as f64 * 2f64.powi((attempt - 1).min(16) as i32) * multiplier;
        let capped = exp.min(self.max.as_millis() as f64);
        // full jitter: uniform in [capped/2, capped]
        let jittered = rand::thread_rng().gen_range((capped / 2.0)..=capped);
        Some(Duration::from_millis(jittered as u64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grows_and_caps() {
        let p = BackoffPolicy { base: Duration::from_millis(100), max: Duration::from_millis(1000), max_retries: 5 };
        let d1 = p.delay_for(1, FailureClass::Transient).unwrap();
        assert!(d1.as_millis() >= 50 && d1.as_millis() <= 100);
        let d5 = p.delay_for(5, FailureClass::Transient).unwrap();
        assert!(d5.as_millis() <= 1000);
        assert!(p.delay_for(6, FailureClass::Transient).is_none());
        assert!(p.delay_for(1, FailureClass::Permanent).is_none());
    }
}
