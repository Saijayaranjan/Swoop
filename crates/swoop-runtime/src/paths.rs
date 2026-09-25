//! Well-known application directories, with the same layout on every platform.

use std::path::{Path, PathBuf};

pub const APP_NAME: &str = "Swoop";
pub const BUNDLE_ID: &str = "app.swoop.desktop";

/// Names used before the product was renamed to Swoop. Only [`migrate_legacy_dir`] needs them.
const LEGACY_ORG_APP: (&str, &str) = ("osprey", "Osprey");
const LEGACY_FILE_STEM: &str = "osprey";

pub struct AppPaths {
    pub data_dir: PathBuf,
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub log_dir: PathBuf,
}

impl AppPaths {
    /// Resolve the per-user directories, honouring `SWOOP_DATA_DIR` for tests and headless
    /// deployments (Docker mounts a volume there).
    pub fn resolve() -> Self {
        if let Ok(base) = std::env::var("SWOOP_DATA_DIR") {
            let base = PathBuf::from(base);
            return Self {
                data_dir: base.clone(),
                config_dir: base.clone(),
                cache_dir: base.join("cache"),
                log_dir: base.join("logs"),
            };
        }
        let dirs = directories::ProjectDirs::from("app", "swoop", APP_NAME);
        if let Some(current) = &dirs {
            static MIGRATE: std::sync::Once = std::sync::Once::new();
            MIGRATE.call_once(|| {
                let (org, app) = LEGACY_ORG_APP;
                if let Some(legacy) = directories::ProjectDirs::from("app", org, app) {
                    migrate_legacy_dir(legacy.data_dir(), current.data_dir());
                    if legacy.config_dir() != legacy.data_dir() {
                        migrate_legacy_dir(legacy.config_dir(), current.config_dir());
                    }
                }
            });
        }
        match dirs {
            Some(d) => Self {
                data_dir: d.data_dir().to_path_buf(),
                config_dir: d.config_dir().to_path_buf(),
                cache_dir: d.cache_dir().to_path_buf(),
                log_dir: d.data_dir().join("logs"),
            },
            None => {
                let base = std::env::temp_dir().join("swoop");
                Self {
                    data_dir: base.clone(),
                    config_dir: base.clone(),
                    cache_dir: base.join("cache"),
                    log_dir: base.join("logs"),
                }
            }
        }
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        for d in [
            &self.data_dir,
            &self.config_dir,
            &self.cache_dir,
            &self.log_dir,
        ] {
            std::fs::create_dir_all(d)?;
        }
        Ok(())
    }

    pub fn database(&self) -> PathBuf {
        self.data_dir.join("swoop.db")
    }
    /// Unix socket path. `sockaddr_un` limits paths to ~104 bytes; when the data directory is
    /// too deep, fall back to a short per-user location derived from the data dir (the per-user
    /// temp dir on macOS is private to the user), so the CLI and server always agree.
    pub fn socket(&self) -> PathBuf {
        let preferred = self.data_dir.join("swoop.sock");
        if preferred.as_os_str().len() < 100 {
            return preferred;
        }
        let digest = blake3::hash(self.data_dir.to_string_lossy().as_bytes()).to_hex();
        let short = std::env::temp_dir().join(format!("swoop-{}.sock", &digest[..12]));
        if short.as_os_str().len() < 100 {
            short
        } else {
            PathBuf::from(format!("/tmp/swoop-{}.sock", &digest[..12]))
        }
    }
    pub fn local_token_file(&self) -> PathBuf {
        self.data_dir.join("local-api.token")
    }
    pub fn torrent_session_dir(&self) -> PathBuf {
        self.data_dir.join("torrents")
    }
    pub fn tls_dir(&self) -> PathBuf {
        self.data_dir.join("tls")
    }
    pub fn plugins_dir(&self) -> PathBuf {
        self.data_dir.join("plugins")
    }

    /// Expand a leading `~` in user-provided paths.
    pub fn expand_home(path: &str) -> PathBuf {
        if let Some(rest) = path.strip_prefix("~/") {
            if let Some(home) = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()) {
                return home.join(rest);
            }
        } else if path == "~" {
            if let Some(home) = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()) {
                return home;
            }
        }
        PathBuf::from(path)
    }

    pub fn default_download_dir() -> PathBuf {
        directories::UserDirs::new()
            .and_then(|u| u.download_dir().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| Self::expand_home("~/Downloads"))
    }
}

/// Best-effort, one-time move of the pre-rename data directory to the Swoop location: only when
/// `legacy` exists, `current` does not, and no running process holds the legacy instance lock.
/// The database (with its WAL/SHM side files) is renamed to `swoop.db`; nothing is ever deleted
/// except the stale lock file and socket. Returns whether the directory was moved.
pub fn migrate_legacy_dir(legacy: &Path, current: &Path) -> bool {
    if !legacy.is_dir() || current.exists() {
        return false;
    }
    let legacy_lock = legacy.join(format!("{LEGACY_FILE_STEM}.lock"));
    let guard = if legacy_lock.exists() {
        match crate::lock::InstanceLock::try_acquire(&legacy_lock) {
            Ok(Some(guard)) => Some(guard),
            // Still in use (or unreadable): leave everything where it is.
            _ => return false,
        }
    } else {
        None
    };
    if let Some(parent) = current.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::rename(legacy, current) {
        tracing::warn!(error = %e, from = %legacy.display(), "could not migrate the old data directory");
        return false;
    }
    drop(guard);
    for suffix in ["", "-wal", "-shm"] {
        let old = current.join(format!("{LEGACY_FILE_STEM}.db{suffix}"));
        let new = current.join(format!("swoop.db{suffix}"));
        if old.exists() && !new.exists() {
            let _ = std::fs::rename(old, new);
        }
    }
    for stale in ["lock", "sock"] {
        let _ = std::fs::remove_file(current.join(format!("{LEGACY_FILE_STEM}.{stale}")));
    }
    tracing::info!(to = %current.display(), "migrated the data directory from the old app name");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_legacy_data_dir_once_without_clobbering() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join("app.osprey.Osprey");
        let current = root.path().join("app.swoop.Swoop");
        std::fs::create_dir_all(legacy.join("logs")).unwrap();
        std::fs::write(legacy.join("osprey.db"), b"db").unwrap();
        std::fs::write(legacy.join("osprey.db-wal"), b"wal").unwrap();
        std::fs::write(legacy.join("local-api.token"), b"t").unwrap();

        assert!(migrate_legacy_dir(&legacy, &current));
        assert!(!legacy.exists());
        assert_eq!(std::fs::read(current.join("swoop.db")).unwrap(), b"db");
        assert_eq!(std::fs::read(current.join("swoop.db-wal")).unwrap(), b"wal");
        assert!(current.join("local-api.token").exists() && current.join("logs").is_dir());

        // A second legacy dir never overwrites an existing Swoop directory.
        std::fs::create_dir_all(&legacy).unwrap();
        assert!(!migrate_legacy_dir(&legacy, &current));
        assert!(legacy.exists());
    }
}
