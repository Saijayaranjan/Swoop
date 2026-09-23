//! Settings: validation, persistence and propagation to the components that cache them
//! (client factory, limiters, torrent engine, notifications).

use crate::engine::Engine;
use osprey_domain::settings::Settings;
use osprey_domain::{DomainError, DomainResult, Event, Notification};
use osprey_runtime::paths::AppPaths;
use std::sync::Arc;

/// Normalise a settings document: expand `~`, validate, clamp.
pub fn normalise(mut s: Settings) -> DomainResult<Settings> {
    s.storage.download_directory =
        AppPaths::expand_home(&s.storage.download_directory.to_string_lossy());
    if !s.storage.download_directory.is_absolute() {
        return Err(DomainError::validation(
            "download_directory must be an absolute path",
        ));
    }
    osprey_runtime::safety::validate_destination_dir(&s.storage.download_directory)
        .map_err(|e| DomainError::validation(e.message))?;
    s.schema_version = osprey_domain::settings::SETTINGS_SCHEMA_VERSION;
    s.validate()?;
    if s.bandwidth.max_active_downloads == 0 {
        s.bandwidth.max_active_downloads = 1;
    }
    if s.bandwidth.max_active_torrents == 0 {
        s.bandwidth.max_active_torrents = 1;
    }
    Ok(s)
}

impl Engine {
    /// Validate, persist, swap and propagate a new settings document.
    pub(crate) async fn update_settings_inner(
        &self,
        settings: Settings,
    ) -> DomainResult<Arc<Settings>> {
        let settings = normalise(settings)?;
        self.store.save_settings(&settings).await?;
        let arc = Arc::new(settings);
        self.apply_settings(arc.clone()).await;
        self.bus
            .publish(Event::SettingsChanged(Box::new((*arc).clone())));
        Ok(arc)
    }

    /// Push a settings snapshot to every component (also used at start).
    pub(crate) async fn apply_settings(&self, settings: Arc<Settings>) {
        let previous = self.settings();
        *self.settings_cell.write() = settings.clone();
        let proxy_url = match &settings.network.global_proxy {
            Some(id) => self.proxy_url_for(id).await,
            None => None,
        };
        self.clients.update_settings(settings.clone(), proxy_url);
        self.bandwidth.apply_settings(&settings);
        let deferred = self.torrent.apply_settings(&settings);
        if !deferred.keys.is_empty() {
            tracing::info!(keys = ?deferred.keys, "torrent settings apply after restart");
        }
        for run in self.tasks.running().into_iter().map(|(_, r)| r) {
            run.limiter.set_burst(settings.bandwidth.burst_bytes);
        }
        if previous.privacy.log_level != settings.privacy.log_level {
            crate::logs::set_level(&settings.privacy.log_level);
        }
        if previous.bandwidth.max_active_downloads != settings.bandwidth.max_active_downloads
            || previous.bandwidth.max_active_torrents != settings.bandwidth.max_active_torrents
        {
            self.admission.notify_one();
        }
    }

    /// Emit a user notification if the settings allow it.
    pub(crate) fn notify(&self, n: Notification) {
        let s = self.settings();
        let f = &s.notifications;
        let allowed = match &n {
            Notification::Completed { .. } => f.completed,
            Notification::Failed { .. } => f.failed,
            Notification::Queued { .. } => f.queued,
            Notification::Scheduled { .. } => f.scheduled,
            Notification::ChecksumMismatch { .. } => f.checksum_mismatch,
            Notification::LowDiskSpace { .. } => f.low_disk_space,
            Notification::TorrentFinished { .. } => f.torrent_finished,
            Notification::DevicePaired { .. } => f.device_paired,
            Notification::AutomationFailed { .. } => f.automation_failure,
            Notification::DuplicateDetected { .. }
            | Notification::UpdateAvailable { .. }
            | Notification::QueueFinished { .. } => true,
        };
        if !allowed {
            return;
        }
        if f.quiet_when_active
            && self
                .active_window
                .load(std::sync::atomic::Ordering::Relaxed)
            && !matches!(
                n,
                Notification::DuplicateDetected { .. } | Notification::DevicePaired { .. }
            )
        {
            return;
        }
        self.bus.publish(Event::Notification(n));
    }
}
