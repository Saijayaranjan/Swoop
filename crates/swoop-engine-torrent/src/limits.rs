//! Seeding policy resolution and rate-limit conversions.
//!
//! Precedence for every knob: engine override set through `set_seeding_limits` → task
//! options → `settings.torrent`.
//!
//! Ratio semantics: `0` = stop as soon as the download completes (no seeding), `< 0` =
//! seed forever, `> 0` = seed until `uploaded / downloaded` reaches the value. Time limit
//! `0` = no time limit.

use librqbit::limits::LimitsConfig;
use std::num::NonZeroU32;
use swoop_domain::settings::TorrentSettings;
use swoop_domain::torrent::SeedingLimits;
use swoop_domain::{Millis, TaskOptions};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeedingPolicy {
    pub seed_when_complete: bool,
    pub ratio_limit: f32,
    pub time_limit_minutes: u32,
    /// Per-torrent upload cap while seeding (bytes/s, 0 = unlimited).
    pub upload_limit: u64,
}

impl SeedingPolicy {
    pub fn resolve(
        options: &TaskOptions,
        override_: Option<&SeedingLimits>,
        settings: &TorrentSettings,
    ) -> Self {
        let o = override_;
        Self {
            seed_when_complete: o
                .and_then(|l| l.seed_when_complete)
                .unwrap_or(settings.seed_when_complete),
            ratio_limit: o
                .and_then(|l| l.ratio_limit)
                .or(options.seed_ratio_limit)
                .unwrap_or(settings.seed_ratio_limit),
            time_limit_minutes: o
                .and_then(|l| l.time_limit_minutes)
                .or(options.seed_time_limit_minutes)
                .unwrap_or(settings.seed_time_limit_minutes),
            upload_limit: o
                .and_then(|l| l.upload_limit)
                .or(options.upload_limit)
                .unwrap_or(0),
        }
    }

    /// Why seeding should stop now, if it should. `downloaded` is the byte count the ratio
    /// is measured against (bytes we hold, so imported data never divides by zero).
    pub fn stop_reason(
        &self,
        uploaded: u64,
        downloaded: u64,
        seeding_since: Option<Millis>,
        now: Millis,
    ) -> Option<&'static str> {
        if !self.seed_when_complete {
            return Some("seeding disabled");
        }
        if self.ratio_limit == 0.0 {
            return Some("ratio limit is 0");
        }
        if self.ratio_limit > 0.0 && ratio(uploaded, downloaded) >= self.ratio_limit {
            return Some("ratio limit reached");
        }
        if self.time_limit_minutes > 0 {
            if let Some(since) = seeding_since {
                let elapsed_min = now.elapsed_since(since) / 60_000;
                if elapsed_min >= i64::from(self.time_limit_minutes) {
                    return Some("seed time limit reached");
                }
            }
        }
        None
    }
}

pub fn ratio(uploaded: u64, downloaded: u64) -> f32 {
    if downloaded == 0 {
        return 0.0;
    }
    (uploaded as f64 / downloaded as f64) as f32
}

/// Bytes/s (0 = unlimited) → librqbit's `Option<NonZeroU32>`, saturating.
pub fn bps(limit: u64) -> Option<NonZeroU32> {
    NonZeroU32::new(u32::try_from(limit).unwrap_or(u32::MAX))
}

pub fn limits_config(download: u64, upload: u64) -> LimitsConfig {
    LimitsConfig {
        download_bps: bps(download),
        upload_bps: bps(upload),
    }
}

/// Session-wide leech-only mode: the user never wants to seed and set no ratio to honour.
pub fn upload_disabled(settings: &TorrentSettings) -> bool {
    !settings.seed_when_complete && settings.seed_ratio_limit == 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_and_stop_reasons() {
        let settings = TorrentSettings {
            seed_ratio_limit: 2.0,
            seed_time_limit_minutes: 0,
            seed_when_complete: true,
            ..Default::default()
        };
        let mut opts = TaskOptions::default();
        let p = SeedingPolicy::resolve(&opts, None, &settings);
        assert_eq!(p.ratio_limit, 2.0);
        assert_eq!(p.stop_reason(100, 100, None, Millis(0)), None);
        assert_eq!(
            p.stop_reason(200, 100, None, Millis(0)),
            Some("ratio limit reached")
        );

        opts.seed_ratio_limit = Some(0.0);
        let p = SeedingPolicy::resolve(&opts, None, &settings);
        assert_eq!(
            p.stop_reason(0, 100, None, Millis(0)),
            Some("ratio limit is 0")
        );

        let over = SeedingLimits {
            ratio_limit: Some(-1.0),
            time_limit_minutes: Some(30),
            ..Default::default()
        };
        let p = SeedingPolicy::resolve(&opts, Some(&over), &settings);
        assert_eq!(p.ratio_limit, -1.0);
        assert_eq!(
            p.stop_reason(10_000, 1, Some(Millis(0)), Millis(29 * 60_000)),
            None
        );
        assert_eq!(
            p.stop_reason(0, 1, Some(Millis(0)), Millis(30 * 60_000)),
            Some("seed time limit reached")
        );

        let no_seed = TorrentSettings {
            seed_when_complete: false,
            seed_ratio_limit: 0.0,
            ..Default::default()
        };
        assert!(upload_disabled(&no_seed));
        assert!(!upload_disabled(&settings));
        assert_eq!(
            SeedingPolicy::resolve(&TaskOptions::default(), None, &no_seed).stop_reason(
                0,
                0,
                None,
                Millis(0)
            ),
            Some("seeding disabled")
        );
    }

    #[test]
    fn conversions() {
        assert_eq!(bps(0), None);
        assert_eq!(bps(1024).map(|n| n.get()), Some(1024));
        assert_eq!(bps(u64::MAX).map(|n| n.get()), Some(u32::MAX));
        assert_eq!(ratio(0, 0), 0.0);
        assert_eq!(ratio(50, 100), 0.5);
        let c = limits_config(10, 0);
        assert_eq!(c.download_bps.map(|n| n.get()), Some(10));
        assert!(c.upload_bps.is_none());
    }
}
