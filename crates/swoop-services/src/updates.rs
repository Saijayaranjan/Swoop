//! Update checks and signed downloads (adapter over `swoop-update`).

use crate::api::UpdateInfo;
use crate::engine::Engine;
use parking_lot::Mutex;
use std::path::PathBuf;
use swoop_domain::{DomainError, DomainResult, Event, Millis, Notification};
use swoop_update::UpdateChecker;

/// Default appcast location; overridable with `SWOOP_UPDATE_FEED_URL`.
pub const DEFAULT_FEED_URL: &str = "https://swoop.app/updates/appcast.json";
/// Environment variable carrying the hex Ed25519 release key (or set at build time).
pub const PUBLIC_KEY_ENV: &str = "SWOOP_UPDATE_PUBLIC_KEY";

/// Last check result plus the feed configuration.
#[derive(Default)]
pub struct UpdateState {
    pub last: Mutex<Option<swoop_update::UpdateInfo>>,
}

fn feed_url() -> String {
    std::env::var("SWOOP_UPDATE_FEED_URL").unwrap_or_else(|_| DEFAULT_FEED_URL.to_owned())
}

fn public_key() -> Option<String> {
    option_env!("SWOOP_UPDATE_PUBLIC_KEY")
        .map(str::to_owned)
        .or_else(|| std::env::var(PUBLIC_KEY_ENV).ok())
        .filter(|k| !k.trim().is_empty())
}

fn to_api(u: &swoop_update::UpdateInfo) -> UpdateInfo {
    UpdateInfo {
        current_version: u.current_version.clone(),
        available: u.available,
        latest_version: u.latest_version.clone(),
        notes: u.notes.clone(),
        download_url: u.download_url.clone(),
        signature_valid: u.signature_valid,
        checked_at: u.checked_at,
    }
}

impl Engine {
    fn checker(&self) -> DomainResult<UpdateChecker> {
        let key = public_key().ok_or_else(|| {
            DomainError::Unavailable("update signing key is not configured".into())
        })?;
        UpdateChecker::new(
            self.clients.clone(),
            feed_url(),
            &key,
            &self.config.app_version,
        )
        .map_err(|e| DomainError::Unavailable(e.message))
    }

    pub(crate) async fn check_for_updates_inner(&self) -> DomainResult<UpdateInfo> {
        let checker = self.checker()?;
        let channel = self.settings().updates.channel.clone();
        let info = checker
            .check(&channel)
            .await
            .map_err(|e| DomainError::Engine(e.message))?;
        let mut s = (*self.settings()).clone();
        s.updates.last_check_at = Some(Millis::now());
        if let Err(e) = self.update_settings_inner(s).await {
            tracing::debug!(error = %e, "last_check_at not persisted");
        }
        let skipped = self.settings().updates.skipped_version.clone();
        let available = info.available && info.latest_version != skipped;
        self.bus.publish(Event::UpdateCheck {
            available,
            version: info.latest_version.clone(),
            notes: info.notes.clone(),
        });
        if available {
            if let Some(v) = &info.latest_version {
                self.notify(Notification::UpdateAvailable {
                    version: v.clone(),
                    notes_url: info.notes_url.clone(),
                });
            }
        }
        *self.updates.last.lock() = Some(info.clone());
        let mut out = to_api(&info);
        out.available = available;
        Ok(out)
    }

    pub(crate) async fn download_update_inner(&self) -> DomainResult<PathBuf> {
        let last = self.updates.last.lock().clone();
        let info = match last {
            Some(i) if i.available => i,
            _ => {
                let _ = self.check_for_updates_inner().await?;
                self.updates
                    .last
                    .lock()
                    .clone()
                    .filter(|i| i.available)
                    .ok_or_else(|| DomainError::Unavailable("no update available".into()))?
            }
        };
        let checker = self.checker()?;
        let dest = self.paths.cache_dir.join("updates");
        tokio::fs::create_dir_all(&dest)
            .await
            .map_err(|e| DomainError::Storage(e.to_string()))?;
        let path = checker
            .download_and_verify(&info, &dest)
            .await
            .map_err(|e| DomainError::Engine(e.message))?;
        let mut verified = info.clone();
        verified.signature_valid = Some(true);
        *self.updates.last.lock() = Some(verified);
        Ok(path)
    }
}
