//! Update checks, signed downloads and (macOS app only) staging: an adapter over `swoop-update`.
//!
//! The REST API and headless servers use [`Engine::check_for_updates_inner`] and
//! [`Engine::download_update_inner`], which report and fetch but never install. The macOS app
//! additionally calls [`Engine::stage_update`] through the FFI, then hands the staged bundle to the
//! `swoop update apply` helper after it quits.

use crate::api::UpdateInfo;
use crate::engine::Engine;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use swoop_domain::{DomainError, DomainResult, Event, Millis, Notification};
use swoop_update::{CheckerOptions, UpdateChecker, VerifiedUpdate};

/// Where the verified update currently is. `phase` is one of `idle`, `downloading`, `verifying`,
/// `verified`, `staging`, `staged`, `failed`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UpdateProgress {
    pub phase: String,
    pub version: Option<String>,
    pub received: u64,
    pub total: Option<u64>,
    pub message: Option<String>,
}

impl Default for UpdateProgress {
    fn default() -> Self {
        Self {
            phase: "idle".into(),
            version: None,
            received: 0,
            total: None,
            message: None,
        }
    }
}

/// Last check, the verified download and live progress.
#[derive(Default)]
pub struct UpdateState {
    pub last: Mutex<Option<swoop_update::UpdateInfo>>,
    verified: Mutex<Option<VerifiedUpdate>>,
    progress: Mutex<UpdateProgress>,
    /// Serialises downloads and staging.
    op: tokio::sync::Mutex<()>,
}

fn to_api(u: &swoop_update::UpdateInfo, available: bool, skipped: bool) -> UpdateInfo {
    UpdateInfo {
        current_version: u.current_version.clone(),
        status: u.status.as_str().to_owned(),
        available,
        skipped,
        latest_version: u.latest_version.clone(),
        name: u.name.clone(),
        notes: u.notes.clone(),
        notes_url: u.notes_url.clone(),
        published_at: u.published_at.clone(),
        prerelease: u.prerelease,
        download_url: u.download_url.clone(),
        size: u.size,
        message: u.message.clone(),
        signature_valid: None,
        checked_at: u.checked_at,
    }
}

impl Engine {
    fn update_checker(&self) -> DomainResult<UpdateChecker> {
        let mut opts = CheckerOptions::for_version(&self.config.app_version);
        opts.cache_path = Some(
            self.paths
                .cache_dir
                .join("updates")
                .join("github-release.json"),
        );
        opts.proxy_url = self.clients.global_proxy_url();
        UpdateChecker::new(opts).map_err(|e| DomainError::Unavailable(e.message))
    }

    fn set_update_progress(&self, f: impl FnOnce(&mut UpdateProgress)) {
        f(&mut self.updates.progress.lock());
    }

    /// Check GitHub. `manual` checks show a version the user skipped; `notify` raises the
    /// "update available" notification (headless/REST) instead of leaving it to the app's UI.
    async fn run_update_check(&self, manual: bool, notify: bool) -> DomainResult<UpdateInfo> {
        let checker = self.update_checker()?;
        let include_pre = self.settings().updates.channel == "beta";
        let info = checker.check(include_pre).await;
        let mut s = (*self.settings()).clone();
        s.updates.last_check_at = Some(Millis::now());
        if let Err(e) = self.update_settings_inner(s).await {
            tracing::debug!(error = %e, "last_check_at not persisted");
        }
        let skipped_version = self.settings().updates.skipped_version.clone();
        let skipped = info.available && info.latest_version == skipped_version;
        let available = info.available && (manual || !skipped);
        self.bus.publish(Event::UpdateCheck {
            available,
            version: info.latest_version.clone(),
            notes: info.notes.clone(),
        });
        if notify && available {
            if let Some(v) = &info.latest_version {
                self.notify(Notification::UpdateAvailable {
                    version: v.clone(),
                    notes_url: info.notes_url.clone(),
                });
            }
        }
        // A different release than the one already downloaded invalidates it.
        {
            let mut verified = self.updates.verified.lock();
            let same = verified.as_ref().map(|v| Some(v.version.to_string()))
                == Some(info.latest_version.clone());
            if !same {
                *verified = None;
            }
        }
        *self.updates.last.lock() = Some(info.clone());
        Ok(to_api(&info, available, skipped))
    }

    /// REST / CLI / headless check: honours "skip this version" and raises a notification.
    pub(crate) async fn check_for_updates_inner(&self) -> DomainResult<UpdateInfo> {
        self.run_update_check(false, true).await
    }

    /// The app's check: quiet (the app shows its own UI); `manual` ignores a skipped version.
    pub async fn check_app_update(&self, manual: bool) -> DomainResult<UpdateInfo> {
        self.run_update_check(manual, false).await
    }

    pub fn update_progress(&self) -> UpdateProgress {
        self.updates.progress.lock().clone()
    }

    /// Download the pending update and verify its Ed25519 signature. Returns the DMG's path.
    pub(crate) async fn download_update_inner(&self) -> DomainResult<PathBuf> {
        let _op = self.updates.op.lock().await;
        let last = self.updates.last.lock().clone();
        let info = match last {
            Some(i) if i.available => i,
            _ => {
                let _ = self.run_update_check(true, false).await?;
                self.updates
                    .last
                    .lock()
                    .clone()
                    .filter(|i| i.available)
                    .ok_or_else(|| DomainError::Unavailable("no update available".into()))?
            }
        };
        let verified = self.updates.verified.lock().clone();
        if let Some(v) = verified {
            if Some(v.version.to_string()) == info.latest_version && v.path.exists() {
                return Ok(v.path);
            }
        }
        let checker = self.update_checker()?;
        let dest = self.paths.cache_dir.join("updates");
        self.set_update_progress(|p| {
            *p = UpdateProgress {
                phase: "downloading".into(),
                version: info.latest_version.clone(),
                total: info.size,
                ..Default::default()
            }
        });
        let progress = |received: u64, total: Option<u64>| {
            let mut p = self.updates.progress.lock();
            p.received = received;
            p.total = total;
            if total.is_some_and(|t| received >= t) {
                p.phase = "verifying".into();
            }
        };
        match checker.download_and_verify(&info, &dest, &progress).await {
            Ok(v) => {
                tracing::info!(version = %v.version, sha256 = %v.sha256_hex, "update downloaded and signature verified");
                let path = v.path.clone();
                // Drop DMGs of older updates.
                if let Ok(entries) = std::fs::read_dir(&dest) {
                    for e in entries.flatten() {
                        let p = e.path();
                        let name = e.file_name().to_string_lossy().into_owned();
                        if p != path && name.starts_with("Swoop-") && name.ends_with(".dmg") {
                            let _ = std::fs::remove_file(p);
                        }
                    }
                }
                self.set_update_progress(|p| {
                    p.phase = "verified".into();
                    p.message = None;
                });
                *self.updates.verified.lock() = Some(v);
                Ok(path)
            }
            Err(e) => {
                tracing::warn!(error = %e.message, "update download refused");
                self.set_update_progress(|p| {
                    p.phase = "failed".into();
                    p.message = Some(e.message.clone());
                });
                Err(DomainError::Engine(e.message))
            }
        }
    }

    /// App only: re-verify the downloaded DMG, mount it read-only, validate the bundle inside
    /// and stage a copy next to `bundle` (the running `Swoop.app`). Returns the staged bundle,
    /// for the `swoop update apply` helper.
    #[cfg(unix)]
    pub async fn stage_update(&self, bundle: PathBuf) -> DomainResult<PathBuf> {
        let _op = self.updates.op.lock().await;
        let verified = self
            .updates
            .verified
            .lock()
            .clone()
            .filter(|v| v.path.exists())
            .ok_or_else(|| DomainError::Unavailable("the update hasn't been downloaded".into()))?;
        let checker = self.update_checker()?;
        let key = *checker.key();
        let current = checker.current_version().clone();
        self.set_update_progress(|p| {
            p.phase = "staging".into();
            p.message = None;
        });
        let result = tokio::task::spawn_blocking(move || {
            swoop_update::install::stage_update(&swoop_update::install::StageRequest {
                dmg: &verified.path,
                signature_b64: &verified.signature_b64,
                key: &key,
                current_version: &current,
                expected_version: Some(&verified.version),
                current_bundle: &bundle,
            })
        })
        .await
        .map_err(|e| DomainError::internal(e.to_string()))?;
        match result {
            Ok(staged) => {
                tracing::info!(staged = %staged.display(), "update verified, mounted, validated and staged");
                self.set_update_progress(|p| p.phase = "staged".into());
                Ok(staged)
            }
            Err(msg) => {
                tracing::warn!(error = %msg, "update refused");
                self.set_update_progress(|p| {
                    p.phase = "failed".into();
                    p.message = Some(msg.clone());
                });
                Err(DomainError::Engine(msg))
            }
        }
    }
}
