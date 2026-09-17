//! Typed application settings. Stored as one JSON document with a schema version so migrations
//! are explicit. Secrets never live here — they are referenced by `CredentialId`.

use crate::queue::TrafficMode;
use crate::{ChecksumAlgorithm, CredentialId, ProxyId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const SETTINGS_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProxyKind {
    #[default]
    Http,
    Https,
    Socks5,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyProfile {
    pub id: ProxyId,
    pub name: String,
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub credential: Option<CredentialId>,
    /// Hosts that bypass the proxy (suffix match), e.g. `localhost`, `*.lan`.
    #[serde(default)]
    pub bypass: Vec<String>,
}

impl ProxyProfile {
    pub fn url(&self, username: Option<&str>, password: Option<&str>) -> String {
        let scheme = match self.kind {
            ProxyKind::Http => "http",
            ProxyKind::Https => "https",
            ProxyKind::Socks5 => "socks5h",
        };
        let auth = match (username, password) {
            (Some(u), Some(p)) => format!("{}:{}@", urlencode(u), urlencode(p)),
            (Some(u), None) => format!("{}@", urlencode(u)),
            _ => String::new(),
        };
        format!("{scheme}://{auth}{}:{}", self.host, self.port)
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkSettings {
    /// Default connections per HTTP/FTP task (adaptive engine may use fewer).
    pub connections_per_task: u8,
    pub max_connections_per_host: u16,
    pub max_total_connections: u16,
    pub adaptive_segmentation: bool,
    /// Minimum segment size in bytes; below this, no further splitting.
    pub min_segment_size: u64,
    pub connect_timeout_seconds: u32,
    pub read_timeout_seconds: u32,
    pub max_retries: u32,
    pub retry_base_delay_ms: u64,
    pub retry_max_delay_ms: u64,
    pub user_agent: String,
    pub follow_redirects: u8,
    pub verify_tls: bool,
    /// Hosts the user explicitly excepted from TLS verification (each shown with a warning).
    pub tls_exceptions: Vec<String>,
    pub prefer_ipv6: bool,
    pub ipv4_only: bool,
    pub global_proxy: Option<ProxyId>,
    pub proxies: Vec<ProxyProfile>,
    /// Custom DNS-over-HTTPS or plain resolvers; empty = system.
    pub dns_servers: Vec<String>,
    pub allow_http2_for_segments: bool,
}

impl Default for NetworkSettings {
    fn default() -> Self {
        Self {
            connections_per_task: 8,
            max_connections_per_host: 16,
            max_total_connections: 64,
            adaptive_segmentation: true,
            min_segment_size: 1024 * 1024,
            connect_timeout_seconds: 20,
            read_timeout_seconds: 45,
            max_retries: 8,
            retry_base_delay_ms: 1_000,
            retry_max_delay_ms: 60_000,
            user_agent: format!("Osprey/{} (+https://osprey.app)", env!("CARGO_PKG_VERSION")),
            follow_redirects: 10,
            verify_tls: true,
            tls_exceptions: Vec::new(),
            prefer_ipv6: false,
            ipv4_only: false,
            global_proxy: None,
            proxies: Vec::new(),
            dns_servers: Vec::new(),
            allow_http2_for_segments: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BandwidthSettings {
    pub mode: TrafficMode,
    /// Used when `mode == Custom`; bytes per second, 0 = unlimited.
    pub custom_download_limit: u64,
    pub custom_upload_limit: u64,
    /// Measured link capacity (bytes/s) used to derive Balanced/Browsing; 0 = unknown.
    pub measured_capacity: u64,
    pub max_active_downloads: u32,
    pub max_active_torrents: u32,
    pub burst_bytes: u64,
}

impl Default for BandwidthSettings {
    fn default() -> Self {
        Self {
            mode: TrafficMode::Unlimited,
            custom_download_limit: 0,
            custom_upload_limit: 0,
            measured_capacity: 0,
            max_active_downloads: 5,
            max_active_torrents: 5,
            burst_bytes: 256 * 1024,
        }
    }
}

impl BandwidthSettings {
    /// Effective global limits (download, upload) in bytes/s; 0 = unlimited.
    pub fn effective_limits(&self) -> (u64, u64) {
        match self.mode {
            TrafficMode::Unlimited | TrafficMode::FullSpeed => (0, 0),
            TrafficMode::Balanced => {
                if self.measured_capacity > 0 {
                    ((self.measured_capacity as f64 * 0.7) as u64, 0)
                } else {
                    (0, 0)
                }
            }
            TrafficMode::Browsing => {
                let cap = if self.measured_capacity > 0 {
                    (self.measured_capacity as f64 * 0.3) as u64
                } else {
                    2 * 1024 * 1024
                };
                (cap.clamp(128 * 1024, 2 * 1024 * 1024), 256 * 1024)
            }
            TrafficMode::Custom => (self.custom_download_limit, self.custom_upload_limit),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageSettings {
    pub download_directory: PathBuf,
    /// Use category subfolders under the download directory.
    pub organise_by_category: bool,
    pub preallocate: bool,
    pub sparse_files: bool,
    /// Bytes to keep free on the destination volume; downloads that would breach it do not start.
    pub reserved_free_space: u64,
    /// Warn before starting downloads larger than this (0 = never).
    pub large_download_warning: u64,
    pub verify_checksum_on_complete: bool,
    pub default_checksum_algorithm: ChecksumAlgorithm,
    /// Detect duplicates by content hash in history.
    pub duplicate_detection: bool,
    pub default_conflict_policy: crate::ConflictPolicy,
    /// Set the quarantine attribute on completed files (macOS).
    pub quarantine_downloads: bool,
    pub temp_suffix: String,
}

impl Default for StorageSettings {
    fn default() -> Self {
        Self {
            download_directory: PathBuf::from("~/Downloads"),
            organise_by_category: false,
            preallocate: true,
            sparse_files: false,
            reserved_free_space: 512 * 1024 * 1024,
            large_download_warning: 5 * 1024 * 1024 * 1024,
            verify_checksum_on_complete: true,
            default_checksum_algorithm: ChecksumAlgorithm::Sha256,
            duplicate_detection: true,
            default_conflict_policy: crate::ConflictPolicy::Ask,
            quarantine_downloads: true,
            temp_suffix: ".osprey-part".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TorrentSettings {
    pub enabled: bool,
    pub listen_port: u16,
    pub dht: bool,
    pub pex: bool,
    pub max_peers_per_torrent: u32,
    pub max_upload_slots: u32,
    pub upload_limit: u64,
    pub download_limit: u64,
    pub seed_ratio_limit: f32,
    pub seed_time_limit_minutes: u32,
    pub seed_when_complete: bool,
    pub sequential_by_default: bool,
    pub announce_interval_seconds: u32,
    /// Extra trackers added to every public torrent.
    pub additional_trackers: Vec<String>,
    /// URL of a curated tracker list to refresh from (e.g. a trackerslist raw URL).
    pub tracker_source_url: Option<String>,
    pub tracker_refresh_hours: u32,
    pub encryption_required: bool,
}

impl Default for TorrentSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            listen_port: 0,
            dht: true,
            pex: true,
            max_peers_per_torrent: 120,
            max_upload_slots: 8,
            upload_limit: 0,
            download_limit: 0,
            seed_ratio_limit: 2.0,
            seed_time_limit_minutes: 0,
            seed_when_complete: true,
            sequential_by_default: false,
            announce_interval_seconds: 0,
            additional_trackers: Vec::new(),
            tracker_source_url: None,
            tracker_refresh_hours: 24,
            encryption_required: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationSettings {
    pub completed: bool,
    pub failed: bool,
    pub queued: bool,
    pub scheduled: bool,
    pub checksum_mismatch: bool,
    pub low_disk_space: bool,
    pub torrent_finished: bool,
    pub device_paired: bool,
    pub automation_failure: bool,
    pub sound: bool,
    /// Suppress notifications while the main window is frontmost.
    pub quiet_when_active: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            completed: true,
            failed: true,
            queued: false,
            scheduled: true,
            checksum_mismatch: true,
            low_disk_space: true,
            torrent_finished: true,
            device_paired: true,
            automation_failure: true,
            sound: true,
            quiet_when_active: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoteSettings {
    /// Remote listener (LAN/Internet) — off by default.
    pub enabled: bool,
    pub bind_address: String,
    pub port: u16,
    pub tls: bool,
    /// Local loopback API for the CLI/extension — always on while the app runs.
    pub local_port: u16,
    pub session_ttl_hours: u32,
    pub max_failed_attempts: u32,
    pub lockout_minutes: u32,
    pub rate_limit_per_minute: u32,
    pub allowed_origins: Vec<String>,
}

impl Default for RemoteSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            bind_address: "0.0.0.0".into(),
            port: 41780,
            tls: true,
            local_port: 41779,
            session_ttl_hours: 24 * 30,
            max_failed_attempts: 5,
            lockout_minutes: 15,
            rate_limit_per_minute: 300,
            allowed_origins: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BrowserSettings {
    pub intercept_downloads: bool,
    /// Minimum size (bytes) for automatic interception; smaller files stay in the browser.
    pub intercept_min_size: u64,
    pub intercept_extensions: Vec<String>,
    pub excluded_domains: Vec<String>,
    pub excluded_url_patterns: Vec<String>,
    pub detect_media: bool,
    pub show_confirmation: bool,
}

impl Default for BrowserSettings {
    fn default() -> Self {
        Self {
            intercept_downloads: true,
            intercept_min_size: 1024 * 1024,
            intercept_extensions: [
                "zip", "rar", "7z", "tar", "gz", "iso", "dmg", "pkg", "exe", "msi", "mp4", "mkv",
                "mp3", "flac", "pdf", "epub", "apk", "deb", "rpm", "torrent",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            excluded_domains: Vec::new(),
            excluded_url_patterns: Vec::new(),
            detect_media: true,
            show_confirmation: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateSettings {
    pub check_automatically: bool,
    pub install_automatically: bool,
    pub channel: String,
    pub last_check_at: Option<crate::Millis>,
    pub skipped_version: Option<String>,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            check_automatically: true,
            install_automatically: false,
            channel: "stable".into(),
            last_check_at: None,
            skipped_version: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrivacySettings {
    /// Opt-in anonymous usage counters (never URLs, never file names).
    pub analytics_opt_in: bool,
    pub keep_history: bool,
    pub history_retention_days: u32,
    pub log_level: String,
    pub log_retention_days: u32,
}

impl Default for PrivacySettings {
    fn default() -> Self {
        Self {
            analytics_opt_in: false,
            keep_history: true,
            history_retention_days: 0,
            log_level: "info".into(),
            log_retention_days: 14,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceSettings {
    /// `system`, `light`, `dark`.
    pub theme: String,
    pub reduced_motion: bool,
    pub compact_rows: bool,
    pub show_speed_graph: bool,
    pub language: String,
    pub show_menu_bar_extra: bool,
    pub show_dock_badge: bool,
    pub launch_at_login: bool,
    pub prevent_sleep_while_active: bool,
    pub confirm_on_quit_with_active: bool,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            reduced_motion: false,
            compact_rows: false,
            show_speed_graph: true,
            language: "en".into(),
            show_menu_bar_extra: true,
            show_dock_badge: true,
            launch_at_login: false,
            prevent_sleep_while_active: true,
            confirm_on_quit_with_active: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub schema_version: u32,
    pub network: NetworkSettings,
    pub bandwidth: BandwidthSettings,
    pub storage: StorageSettings,
    pub torrent: TorrentSettings,
    pub notifications: NotificationSettings,
    pub remote: RemoteSettings,
    pub browser: BrowserSettings,
    pub updates: UpdateSettings,
    pub privacy: PrivacySettings,
    pub appearance: AppearanceSettings,
    /// Free-form plugin settings keyed by plugin id.
    pub plugins: BTreeMap<String, serde_json::Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            network: NetworkSettings::default(),
            bandwidth: BandwidthSettings::default(),
            storage: StorageSettings::default(),
            torrent: TorrentSettings::default(),
            notifications: NotificationSettings::default(),
            remote: RemoteSettings::default(),
            browser: BrowserSettings::default(),
            updates: UpdateSettings::default(),
            privacy: PrivacySettings::default(),
            appearance: AppearanceSettings::default(),
            plugins: BTreeMap::new(),
        }
    }
}

impl Settings {
    /// Validate invariants that the UI/API must not be able to violate.
    pub fn validate(&self) -> Result<(), crate::DomainError> {
        let n = &self.network;
        if n.connections_per_task == 0 || n.connections_per_task > 64 {
            return Err(crate::DomainError::validation(
                "connections_per_task must be 1..=64",
            ));
        }
        if n.max_connections_per_host == 0 {
            return Err(crate::DomainError::validation(
                "max_connections_per_host must be > 0",
            ));
        }
        if n.min_segment_size < 64 * 1024 {
            return Err(crate::DomainError::validation(
                "min_segment_size must be >= 64 KiB",
            ));
        }
        if self.remote.port == 0 || self.remote.local_port == 0 {
            return Err(crate::DomainError::validation("ports must be non-zero"));
        }
        if self.remote.port == self.remote.local_port {
            return Err(crate::DomainError::validation(
                "remote and local ports must differ",
            ));
        }
        if self.storage.temp_suffix.is_empty() || self.storage.temp_suffix.contains('/') {
            return Err(crate::DomainError::validation(
                "temp_suffix must be a non-empty suffix",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_validate() {
        Settings::default().validate().unwrap();
    }

    #[test]
    fn traffic_modes() {
        let mut b = BandwidthSettings {
            measured_capacity: 10_000_000,
            ..Default::default()
        };
        b.mode = TrafficMode::Balanced;
        assert_eq!(b.effective_limits().0, 7_000_000);
        b.mode = TrafficMode::Browsing;
        assert_eq!(b.effective_limits().0, 2 * 1024 * 1024);
        b.mode = TrafficMode::Custom;
        b.custom_download_limit = 42;
        assert_eq!(b.effective_limits(), (42, 0));
    }

    #[test]
    fn proxy_url_encodes_credentials() {
        let p = ProxyProfile {
            id: ProxyId::new(),
            name: "x".into(),
            kind: ProxyKind::Socks5,
            host: "127.0.0.1".into(),
            port: 1080,
            credential: None,
            bypass: vec![],
        };
        assert_eq!(
            p.url(Some("a b"), Some("p@ss")),
            "socks5h://a%20b:p%40ss@127.0.0.1:1080"
        );
    }
}
