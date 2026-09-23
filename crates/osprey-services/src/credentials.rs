//! Credential storage: secrets live in the OS keychain (`keyring`, service
//! `app.osprey.desktop`, account = credential id); the store keeps only the index (name,
//! username). When no keychain is available (headless Linux without Secret Service) a
//! JSON file with mode 0600 in the data directory is used instead, with a warning.

use crate::engine::Engine;
use osprey_domain::{CredentialId, DomainError, DomainResult};
use osprey_runtime::engine::TransferSecrets;
use osprey_runtime::paths::BUNDLE_ID;
use parking_lot::Mutex;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// Environment variable forcing the file store (tests, containers).
pub const FILE_STORE_ENV: &str = "OSPREY_CREDENTIALS_FILE_STORE";

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
struct FileEntry {
    username: Option<String>,
    secret: String,
}

/// Where secrets go.
pub struct CredentialStore {
    file: PathBuf,
    /// Once the keyring failed we stay on the file store for the process lifetime.
    use_file: AtomicBool,
    warned: AtomicBool,
    file_lock: Mutex<()>,
}

impl CredentialStore {
    pub fn new(data_dir: &std::path::Path) -> Self {
        let forced = std::env::var(FILE_STORE_ENV)
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        Self {
            file: data_dir.join("credentials.json"),
            use_file: AtomicBool::new(forced),
            warned: AtomicBool::new(false),
            file_lock: Mutex::new(()),
        }
    }

    fn warn_fallback(&self, reason: &str) {
        if !self.warned.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                reason,
                path = %self.file.display(),
                "OS keychain unavailable; storing credentials in a 0600 file"
            );
        }
        self.use_file.store(true, Ordering::Relaxed);
    }

    fn read_file(&self) -> BTreeMap<String, FileEntry> {
        let _g = self.file_lock.lock();
        match std::fs::read(&self.file) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => BTreeMap::new(),
        }
    }

    fn write_file(&self, map: &BTreeMap<String, FileEntry>) -> DomainResult<()> {
        let _g = self.file_lock.lock();
        let bytes = serde_json::to_vec_pretty(map).map_err(DomainError::internal)?;
        let tmp = self.file.with_extension("json.tmp");
        std::fs::write(&tmp, bytes).map_err(|e| DomainError::Storage(e.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| DomainError::Storage(e.to_string()))?;
        }
        std::fs::rename(&tmp, &self.file).map_err(|e| DomainError::Storage(e.to_string()))?;
        Ok(())
    }

    /// Store `secret` under `id` (blocking; call from `spawn_blocking`).
    pub fn put(&self, id: &CredentialId, username: Option<&str>, secret: &str) -> DomainResult<()> {
        if !self.use_file.load(Ordering::Relaxed) {
            match keyring::Entry::new(BUNDLE_ID, id.as_str()).and_then(|e| e.set_password(secret)) {
                Ok(()) => return Ok(()),
                Err(e) => self.warn_fallback(&e.to_string()),
            }
        }
        let mut map = self.read_file();
        map.insert(
            id.0.clone(),
            FileEntry {
                username: username.map(str::to_owned),
                secret: secret.to_owned(),
            },
        );
        self.write_file(&map)
    }

    /// Fetch a secret (blocking).
    pub fn get(&self, id: &CredentialId) -> Option<String> {
        if !self.use_file.load(Ordering::Relaxed) {
            match keyring::Entry::new(BUNDLE_ID, id.as_str()).and_then(|e| e.get_password()) {
                Ok(p) => return Some(p),
                Err(keyring::Error::NoEntry) => {}
                Err(e) => self.warn_fallback(&e.to_string()),
            }
        }
        self.read_file().get(id.as_str()).map(|e| e.secret.clone())
    }

    /// Remove a secret (blocking). Missing entries are not an error.
    pub fn delete(&self, id: &CredentialId) -> DomainResult<()> {
        if !self.use_file.load(Ordering::Relaxed) {
            match keyring::Entry::new(BUNDLE_ID, id.as_str()).and_then(|e| e.delete_credential()) {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(e) => self.warn_fallback(&e.to_string()),
            }
        }
        let mut map = self.read_file();
        if map.remove(id.as_str()).is_some() {
            self.write_file(&map)?;
        }
        Ok(())
    }
}

impl Engine {
    pub(crate) async fn store_credential_inner(
        &self,
        name: String,
        username: Option<String>,
        secret: String,
    ) -> DomainResult<CredentialId> {
        let name = name.trim().to_owned();
        if name.is_empty() {
            return Err(DomainError::validation("credential name must not be empty"));
        }
        if secret.is_empty() {
            return Err(DomainError::validation("secret must not be empty"));
        }
        let id = CredentialId::new();
        let this = self.this.clone();
        let (id2, user2) = (id.clone(), username.clone());
        tokio::task::spawn_blocking(move || {
            let e = this
                .upgrade()
                .ok_or_else(|| DomainError::Unavailable("engine stopped".into()))?;
            e.credentials.put(&id2, user2.as_deref(), &secret)
        })
        .await
        .map_err(DomainError::internal)??;
        self.store
            .upsert_credential_meta(&id, &name, username.as_deref())
            .await?;
        Ok(id)
    }

    pub(crate) async fn list_credentials_inner(
        &self,
    ) -> DomainResult<Vec<(CredentialId, String, Option<String>)>> {
        Ok(self
            .store
            .list_credential_meta()
            .await?
            .into_iter()
            .map(|m| (m.id, m.name, m.username))
            .collect())
    }

    pub(crate) async fn delete_credential_inner(&self, id: CredentialId) -> DomainResult<()> {
        let this = self.this.clone();
        let id2 = id.clone();
        tokio::task::spawn_blocking(move || {
            let e = this
                .upgrade()
                .ok_or_else(|| DomainError::Unavailable("engine stopped".into()))?;
            e.credentials.delete(&id2)
        })
        .await
        .map_err(DomainError::internal)??;
        if !self.store.delete_credential_meta(&id).await? {
            return Err(DomainError::not_found(format!("credential {id}")));
        }
        Ok(())
    }

    /// Fetch a credential's (username, secret) pair.
    pub(crate) async fn credential_pair(
        &self,
        id: &CredentialId,
    ) -> Option<(Option<String>, String)> {
        let meta = self.store.list_credential_meta().await.ok()?;
        let username = meta.iter().find(|m| &m.id == id)?.username.clone();
        let this = self.this.clone();
        let id2 = id.clone();
        let secret = tokio::task::spawn_blocking(move || {
            this.upgrade().and_then(|e| e.credentials.get(&id2))
        })
        .await
        .ok()??;
        Some((username, secret))
    }

    /// Resolve the secrets a run needs: credential → basic/bearer, proxy profile → proxy URL.
    pub(crate) async fn resolve_secrets(&self, task: &osprey_domain::Task) -> TransferSecrets {
        let mut secrets = TransferSecrets::default();
        if let Some(cid) = &task.options.credential {
            if let Some((username, secret)) = self.credential_pair(cid).await {
                match username {
                    Some(u) => {
                        secrets.username = Some(u);
                        secrets.password = Some(secret);
                    }
                    None => secrets
                        .headers
                        .push(("Authorization".into(), format!("Bearer {secret}"))),
                }
            }
        }
        let settings = self.settings();
        let proxy_id = task
            .options
            .proxy
            .clone()
            .or_else(|| settings.network.global_proxy.clone());
        if let (Some(pid), false) = (proxy_id, task.options.direct_connection) {
            secrets.proxy_url = self.proxy_url_for(&pid).await;
        }
        secrets
    }

    /// Proxy URL (with credentials) for a profile id.
    pub(crate) async fn proxy_url_for(&self, id: &osprey_domain::ProxyId) -> Option<String> {
        let settings = self.settings();
        let profile = settings
            .network
            .proxies
            .iter()
            .find(|p| &p.id == id)?
            .clone();
        let creds = match &profile.credential {
            Some(c) => self.credential_pair(c).await,
            None => None,
        };
        Some(match creds {
            Some((u, p)) => profile.url(u.as_deref(), Some(&p)),
            None => profile.url(None, None),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_store_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = CredentialStore::new(dir.path());
        store.use_file.store(true, Ordering::Relaxed);
        let id = CredentialId::new();
        store.put(&id, Some("bob"), "hunter2").unwrap();
        assert_eq!(store.get(&id).as_deref(), Some("hunter2"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("credentials.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        store.delete(&id).unwrap();
        assert!(store.get(&id).is_none());
    }
}
