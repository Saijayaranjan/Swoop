//! Download Health Score: a transparent 0–100 indicator derived from measurable data.
//! The inputs are always exposed next to the score so it is never a black box.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct HealthScore {
    /// 0–100.
    pub score: u8,
    /// 0–100 each; the score is a weighted blend of these.
    pub source_stability: u8,
    pub throughput_consistency: u8,
    pub connection_quality: u8,
    pub retry_pressure: u8,
    pub remaining_risk: u8,
    /// Short explanation keys the UI localises (`health.throttled`, …).
    pub notes: Vec<String>,
}

/// Raw measurements the score is computed from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HealthInputs {
    pub retries: u32,
    pub failed_connections: u32,
    pub successful_connections: u32,
    pub throughput_drops: u32,
    pub samples: u32,
    pub range_supported: Option<bool>,
    pub mirrors_switched: u32,
    pub throttled_events: u32,
    pub fraction_done: f32,
    pub speed_cv: f32, // coefficient of variation of speed samples
    pub active_seconds: u64,
}

impl HealthScore {
    pub fn compute(i: &HealthInputs) -> Self {
        let mut notes = Vec::new();

        let total_conn = i.failed_connections + i.successful_connections;
        let connection_quality = if total_conn == 0 {
            80
        } else {
            let ok = i.successful_connections as f32 / total_conn as f32;
            (ok * 100.0).round() as u8
        };
        if connection_quality < 60 {
            notes.push("health.connections_failing".into());
        }

        let retry_pressure = 100u32.saturating_sub(i.retries.saturating_mul(12)).min(100) as u8;
        if i.retries >= 3 {
            notes.push("health.many_retries".into());
        }

        let throughput_consistency = {
            let cv_penalty = (i.speed_cv.clamp(0.0, 2.0) * 35.0) as u32;
            let drop_penalty = i.throughput_drops.min(10) * 6;
            100u32.saturating_sub(cv_penalty + drop_penalty).min(100) as u8
        };
        if i.throttled_events > 0 {
            notes.push("health.throttled".into());
        }

        let source_stability = {
            let mut s: i32 = 90;
            if i.range_supported == Some(false) {
                s -= 25;
                notes.push("health.no_range".into());
            }
            s -= (i.mirrors_switched.min(5) * 8) as i32;
            s -= (i.throttled_events.min(5) * 6) as i32;
            s.clamp(0, 100) as u8
        };

        let remaining_risk = {
            // Risk shrinks as the download approaches completion, grows with instability.
            let remaining = (1.0 - i.fraction_done.clamp(0.0, 1.0)) * 100.0;
            let instability = (100 - throughput_consistency as i32).max(0) as f32;
            ((remaining * 0.5 + instability * 0.5).round() as i32).clamp(0, 100) as u8
        };

        let score = (source_stability as f32 * 0.3
            + throughput_consistency as f32 * 0.25
            + connection_quality as f32 * 0.25
            + retry_pressure as f32 * 0.2)
            .round()
            .clamp(0.0, 100.0) as u8;

        Self {
            score,
            source_stability,
            throughput_consistency,
            connection_quality,
            retry_pressure,
            remaining_risk,
            notes,
        }
    }

    pub fn label_key(&self) -> &'static str {
        match self.score {
            85..=100 => "health.excellent",
            65..=84 => "health.good",
            40..=64 => "health.fair",
            _ => "health.poor",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healthy_download_scores_high() {
        let h = HealthScore::compute(&HealthInputs {
            successful_connections: 8,
            samples: 50,
            fraction_done: 0.5,
            ..Default::default()
        });
        assert!(h.score >= 85, "{h:?}");
    }

    #[test]
    fn unstable_download_scores_low() {
        let h = HealthScore::compute(&HealthInputs {
            retries: 6,
            failed_connections: 9,
            successful_connections: 1,
            throughput_drops: 8,
            speed_cv: 1.5,
            range_supported: Some(false),
            throttled_events: 3,
            ..Default::default()
        });
        assert!(h.score < 40, "{h:?}");
        assert!(h.notes.contains(&"health.no_range".to_string()));
    }
}
