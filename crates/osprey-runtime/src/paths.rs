//! Well-known application directories, with the same layout on every platform.

use std::path::PathBuf;

pub const APP_NAME: &str = "Osprey";
pub const BUNDLE_ID: &str = "app.osprey.desktop";

pub struct AppPaths {
    pub data_dir: PathBuf,
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub log_dir: PathBuf,
}

impl AppPaths {
    /// Resolve the per-user directories, honouring `OSPREY_DATA_DIR` for tests and headless
    /// deployments (Docker mounts a volume there).
    pub fn resolve() -> Self {
        if let Ok(base) = std::env::var("OSPREY_DATA_DIR") {
            let base = PathBuf::from(base);
            return Self { data_dir: base.clone(), config_dir: base.clone(), cache_dir: base.join("cache"), log_dir: base.join("logs") };
        }
        let dirs = directories::ProjectDirs::from("app", "osprey", APP_NAME);
        match dirs {
            Some(d) => Self {
                data_dir: d.data_dir().to_path_buf(),
                config_dir: d.config_dir().to_path_buf(),
                cache_dir: d.cache_dir().to_path_buf(),
                log_dir: d.data_dir().join("logs"),
            },
            None => {
                let base = std::env::temp_dir().join("osprey");
                Self { data_dir: base.clone(), config_dir: base.clone(), cache_dir: base.join("cache"), log_dir: base.join("logs") }
            }
        }
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        for d in [&self.data_dir, &self.config_dir, &self.cache_dir, &self.log_dir] {
            std::fs::create_dir_all(d)?;
        }
        Ok(())
    }

    pub fn database(&self) -> PathBuf {
        self.data_dir.join("osprey.db")
    }
    pub fn socket(&self) -> PathBuf {
        self.data_dir.join("osprey.sock")
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
