//! Scheduler model: when queues/tasks may run and what to do when a window opens or closes.

use crate::{Millis, QueueId, ScheduleId};
use chrono::{Datelike, Local, NaiveTime, TimeZone, Timelike, Weekday};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Recurrence {
    /// Runs once at `at` (unix millis) — e.g. "start tonight at 23:00".
    Once { at: Millis },
    /// Every day between `start` and `end` (local time, `HH:MM`). Windows may cross midnight.
    Daily { start: String, end: String },
    /// Selected weekdays (0 = Monday … 6 = Sunday).
    Weekly { days: Vec<u8>, start: String, end: String },
    /// Between two absolute instants.
    Range { from: Millis, to: Millis },
    /// Always active; used for condition-only schedules ("only on AC power").
    Always,
}

impl Recurrence {
    pub fn weekdays() -> Self {
        Recurrence::Weekly { days: vec![0, 1, 2, 3, 4], start: "00:00".into(), end: "23:59".into() }
    }
    pub fn weekends() -> Self {
        Recurrence::Weekly { days: vec![5, 6], start: "00:00".into(), end: "23:59".into() }
    }
}

/// Environmental conditions evaluated by the scheduler. The platform layer supplies the
/// measurements ([`EnvironmentSnapshot`]); the logic is shared.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Condition {
    NetworkAvailable,
    /// Network interface is not marked expensive/metered (cellular hotspot).
    NotMetered,
    OnAcPower,
    BatteryAbove { percent: u8 },
    /// Current global download speed below this many bytes/s (leave room for other traffic).
    BandwidthBelow { bytes_per_second: u64 },
    VpnActive { active: bool },
    ActiveTransfersBelow { count: u32 },
    /// The Wi-Fi SSID matches.
    NetworkNamed { ssid: String },
}

/// Measurements supplied by the platform layer (Swift on macOS; a small probe in headless mode).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct EnvironmentSnapshot {
    pub network_available: bool,
    pub metered: bool,
    pub on_ac_power: bool,
    pub battery_percent: Option<u8>,
    pub vpn_active: bool,
    pub ssid: Option<String>,
    pub download_speed: u64,
    pub active_transfers: u32,
    pub at: Millis,
}

impl EnvironmentSnapshot {
    pub fn assume_desktop() -> Self {
        Self { network_available: true, on_ac_power: true, at: Millis::now(), ..Default::default() }
    }
}

impl Condition {
    pub fn holds(&self, env: &EnvironmentSnapshot) -> bool {
        match self {
            Condition::NetworkAvailable => env.network_available,
            Condition::NotMetered => !env.metered,
            Condition::OnAcPower => env.on_ac_power,
            Condition::BatteryAbove { percent } => env.battery_percent.map(|b| b > *percent).unwrap_or(true),
            Condition::BandwidthBelow { bytes_per_second } => env.download_speed < *bytes_per_second,
            Condition::VpnActive { active } => env.vpn_active == *active,
            Condition::ActiveTransfersBelow { count } => env.active_transfers < *count,
            Condition::NetworkNamed { ssid } => env.ssid.as_deref().map(|s| s.eq_ignore_ascii_case(ssid)).unwrap_or(false),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ScheduleAction {
    StartQueue { queue_id: QueueId },
    PauseQueue { queue_id: QueueId },
    SetSpeedLimit { download: u64, upload: u64 },
    SetConnectionLimit { per_task: u8 },
    SetTrafficMode { mode: crate::queue::TrafficMode },
    /// Executed by the app layer (never by the core).
    LaunchApplication { path: String },
    RunAutomation { automation_id: String },
    Notify { message: String },
    /// Executed by the app layer after all transfers are paused.
    SleepComputer,
    QuitApplication,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Schedule {
    pub id: ScheduleId,
    pub name: String,
    pub enabled: bool,
    pub recurrence: Recurrence,
    #[serde(default)]
    pub conditions: Vec<Condition>,
    /// Run when the window opens (or conditions become true).
    #[serde(default)]
    pub on_start: Vec<ScheduleAction>,
    /// Run when the window closes (or conditions become false).
    #[serde(default)]
    pub on_end: Vec<ScheduleAction>,
    /// Pause attached tasks/queues while the window is closed (the common "night queue" case).
    #[serde(default = "default_true")]
    pub gate_attached: bool,
    #[serde(default)]
    pub last_fired_at: Option<Millis>,
    pub created_at: Millis,
    pub updated_at: Millis,
}

fn default_true() -> bool {
    true
}

impl Schedule {
    pub fn new(name: impl Into<String>, recurrence: Recurrence) -> Self {
        let now = Millis::now();
        Self {
            id: ScheduleId::new(),
            name: name.into(),
            enabled: true,
            recurrence,
            conditions: Vec::new(),
            on_start: Vec::new(),
            on_end: Vec::new(),
            gate_attached: true,
            last_fired_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Is the time window open at `now` (local time)?
    pub fn window_open_at(&self, now: Millis) -> bool {
        let local = Local.timestamp_millis_opt(now.0).single().unwrap_or_else(Local::now);
        match &self.recurrence {
            Recurrence::Always => true,
            Recurrence::Once { at } => now.0 >= at.0 && now.0 < at.0 + 24 * 3600 * 1000,
            Recurrence::Range { from, to } => now.0 >= from.0 && now.0 < to.0,
            Recurrence::Daily { start, end } => time_in_window(local.time(), start, end),
            Recurrence::Weekly { days, start, end } => {
                let wd = weekday_index(local.weekday());
                // A window that crosses midnight belongs to the day it started on.
                let (s, e) = (parse_hhmm(start), parse_hhmm(end));
                let crosses = matches!((s, e), (Some(s), Some(e)) if e <= s);
                let today = days.contains(&wd) && time_in_window(local.time(), start, end);
                if today {
                    return true;
                }
                if crosses {
                    let yesterday = (wd + 6) % 7;
                    if days.contains(&yesterday) {
                        if let Some(e) = e {
                            return local.time() < e;
                        }
                    }
                }
                false
            }
        }
    }

    /// Fully active = window open and all conditions hold.
    pub fn is_active(&self, now: Millis, env: &EnvironmentSnapshot) -> bool {
        self.enabled && self.window_open_at(now) && self.conditions.iter().all(|c| c.holds(env))
    }

    /// Next instant (millis) at which the window state may change; used to arm a timer.
    pub fn next_boundary_after(&self, now: Millis) -> Option<Millis> {
        let local = Local.timestamp_millis_opt(now.0).single()?;
        match &self.recurrence {
            Recurrence::Always => None,
            Recurrence::Once { at } => {
                if now.0 < at.0 {
                    Some(*at)
                } else {
                    Some(Millis(at.0 + 24 * 3600 * 1000))
                }
            }
            Recurrence::Range { from, to } => {
                if now.0 < from.0 {
                    Some(*from)
                } else if now.0 < to.0 {
                    Some(*to)
                } else {
                    None
                }
            }
            Recurrence::Daily { start, end } | Recurrence::Weekly { start, end, .. } => {
                let (s, e) = (parse_hhmm(start)?, parse_hhmm(end)?);
                let today = local.date_naive();
                let mut candidates = Vec::new();
                for d in 0..=7i64 {
                    let day = today + chrono::Duration::days(d);
                    for t in [s, e] {
                        if let Some(dt) = Local.from_local_datetime(&day.and_time(t)).single() {
                            let ms = dt.timestamp_millis();
                            if ms > now.0 {
                                candidates.push(ms);
                            }
                        }
                    }
                }
                candidates.into_iter().min().map(Millis)
            }
        }
    }
}

fn weekday_index(w: Weekday) -> u8 {
    w.num_days_from_monday() as u8
}

fn parse_hhmm(s: &str) -> Option<NaiveTime> {
    let (h, m) = s.trim().split_once(':')?;
    NaiveTime::from_hms_opt(h.parse().ok()?, m.parse().ok()?, 0)
}

/// `start <= t < end`, with windows that cross midnight (`23:00`–`06:00`).
fn time_in_window(t: NaiveTime, start: &str, end: &str) -> bool {
    let (Some(s), Some(e)) = (parse_hhmm(start), parse_hhmm(end)) else {
        return false;
    };
    let secs = |x: NaiveTime| x.hour() * 3600 + x.minute() * 60;
    let (t, s, e) = (secs(t), secs(s), secs(e));
    if e == s {
        return true; // 24h window
    }
    if e > s {
        t >= s && t < e
    } else {
        t >= s || t < e
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_cross_midnight() {
        let t = |h, m| NaiveTime::from_hms_opt(h, m, 0).unwrap();
        assert!(time_in_window(t(1, 0), "23:00", "06:00"));
        assert!(time_in_window(t(23, 30), "23:00", "06:00"));
        assert!(!time_in_window(t(12, 0), "23:00", "06:00"));
        assert!(time_in_window(t(12, 0), "09:00", "17:00"));
        assert!(!time_in_window(t(17, 0), "09:00", "17:00"));
    }

    #[test]
    fn conditions() {
        let env = EnvironmentSnapshot { network_available: true, on_ac_power: false, battery_percent: Some(40), ..Default::default() };
        assert!(Condition::NetworkAvailable.holds(&env));
        assert!(!Condition::OnAcPower.holds(&env));
        assert!(Condition::BatteryAbove { percent: 30 }.holds(&env));
        assert!(!Condition::BatteryAbove { percent: 50 }.holds(&env));
    }

    #[test]
    fn once_and_range() {
        let now = Millis(1_000_000);
        let s = Schedule::new("once", Recurrence::Once { at: Millis(2_000_000) });
        assert!(!s.window_open_at(now));
        assert!(s.window_open_at(Millis(2_000_001)));
        assert_eq!(s.next_boundary_after(now), Some(Millis(2_000_000)));
        let r = Schedule::new("range", Recurrence::Range { from: Millis(10), to: Millis(20) });
        assert!(r.window_open_at(Millis(15)));
        assert!(!r.window_open_at(Millis(25)));
    }
}
