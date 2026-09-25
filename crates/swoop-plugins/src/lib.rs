//! Plugin host: discovers `plugins/<id>/swoop-plugin.json` manifests, tracks enablement and
//! granted permissions in `plugins/state.json`, and enforces that a plugin only ever receives
//! the permissions the user granted. Execution of plugin code is out-of-process and out of scope
//! for this version; the host provides the manifest/permission model the UI and API expose.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use swoop_domain::{ErrorKind, TaskError};

pub const KNOWN_PERMISSIONS: &[&str] = &[
    "network",
    "filesystem:read",
    "filesystem:write",
    "tasks:read",
    "tasks:add",
    "tasks:control",
    "notifications",
    "media:extract",
    "protocol",
];
pub const KINDS: &[&str] = &[
    "protocol",
    "extractor",
    "naming",
    "automation",
    "metadata",
    "integration",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    pub kind: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub path: PathBuf,
    #[serde(default)]
    pub granted_permissions: Vec<String>,
    /// Entry point relative to the plugin directory (executable or script), informational.
    #[serde(default)]
    pub entry: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct State {
    plugins: BTreeMap<String, PluginState>,
}
#[derive(Default, Serialize, Deserialize, Clone)]
struct PluginState {
    enabled: bool,
    granted: Vec<String>,
}

pub struct PluginHost {
    dir: PathBuf,
}

impl PluginHost {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn state_path(&self) -> PathBuf {
        self.dir.join("state.json")
    }
    fn load_state(&self) -> State {
        std::fs::read(self.state_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    fn save_state(&self, s: &State) -> Result<(), TaskError> {
        std::fs::create_dir_all(&self.dir).map_err(|e| TaskError::from_io(&e, "plugins dir"))?;
        let tmp = self.state_path().with_extension("json.tmp");
        std::fs::write(
            &tmp,
            serde_json::to_vec_pretty(s).map_err(|e| TaskError::internal(e.to_string()))?,
        )
        .map_err(|e| TaskError::from_io(&e, "write state"))?;
        std::fs::rename(&tmp, self.state_path()).map_err(|e| TaskError::from_io(&e, "rename state"))
    }

    fn validate(m: &PluginManifest) -> Result<(), TaskError> {
        if m.id.is_empty()
            || !m
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_')
            || m.id.len() > 64
        {
            return Err(TaskError::new(ErrorKind::ParseError, "invalid plugin id"));
        }
        semver::Version::parse(&m.version)
            .map_err(|_| TaskError::new(ErrorKind::ParseError, "plugin version must be semver"))?;
        if !KINDS.contains(&m.kind.as_str()) {
            return Err(TaskError::new(
                ErrorKind::ParseError,
                format!("unknown plugin kind {}", m.kind),
            ));
        }
        for p in &m.permissions {
            if !KNOWN_PERMISSIONS.contains(&p.as_str()) {
                return Err(TaskError::new(
                    ErrorKind::ParseError,
                    format!("unknown permission {p}"),
                ));
            }
        }
        Ok(())
    }

    /// Discover manifests. Invalid manifests are skipped with a warning.
    pub fn scan(&self) -> Vec<PluginManifest> {
        let state = self.load_state();
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return out;
        };
        for entry in rd.flatten() {
            let p = entry.path();
            let manifest = p.join("swoop-plugin.json");
            if !manifest.is_file() {
                continue;
            }
            let Ok(bytes) = std::fs::read(&manifest) else {
                continue;
            };
            let Ok(mut m) = serde_json::from_slice::<PluginManifest>(&bytes) else {
                tracing::warn!(path = %manifest.display(), "invalid plugin manifest");
                continue;
            };
            if let Err(e) = Self::validate(&m) {
                tracing::warn!(path = %manifest.display(), "rejected plugin: {}", e.message);
                continue;
            }
            if p.file_name().and_then(|n| n.to_str()) != Some(m.id.as_str()) {
                tracing::warn!(path = %manifest.display(), "plugin directory name must equal its id");
                continue;
            }
            m.path = p.clone();
            let st = state.plugins.get(&m.id).cloned().unwrap_or_default();
            // never grant more than the manifest declares
            m.granted_permissions = st
                .granted
                .into_iter()
                .filter(|g| m.permissions.contains(g))
                .collect();
            m.enabled = st.enabled
                && m.permissions
                    .iter()
                    .all(|p| m.granted_permissions.contains(p));
            out.push(m);
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn get(&self, id: &str) -> Option<PluginManifest> {
        self.scan().into_iter().find(|m| m.id == id)
    }

    /// Enable requires every declared permission to be granted explicitly.
    pub fn set_enabled(
        &self,
        id: &str,
        enabled: bool,
        granted: Vec<String>,
    ) -> Result<PluginManifest, TaskError> {
        let m = self
            .get(id)
            .ok_or_else(|| TaskError::new(ErrorKind::NotFound, format!("plugin {id}")))?;
        let granted: Vec<String> = granted
            .into_iter()
            .filter(|g| m.permissions.contains(g))
            .collect();
        if enabled && !m.permissions.iter().all(|p| granted.contains(p)) {
            return Err(TaskError::new(
                ErrorKind::PermissionDenied,
                "all declared permissions must be granted to enable a plugin",
            ));
        }
        let mut s = self.load_state();
        s.plugins
            .insert(id.to_owned(), PluginState { enabled, granted });
        self.save_state(&s)?;
        self.get(id)
            .ok_or_else(|| TaskError::internal("plugin vanished"))
    }

    pub fn uninstall(&self, id: &str) -> Result<(), TaskError> {
        let m = self
            .get(id)
            .ok_or_else(|| TaskError::new(ErrorKind::NotFound, format!("plugin {id}")))?;
        let canon_dir =
            std::fs::canonicalize(&self.dir).map_err(|e| TaskError::from_io(&e, "plugins dir"))?;
        let canon =
            std::fs::canonicalize(&m.path).map_err(|e| TaskError::from_io(&e, "plugin path"))?;
        if !canon.starts_with(&canon_dir) || canon == canon_dir {
            return Err(TaskError::new(
                ErrorKind::PathTraversal,
                "plugin path outside plugins dir",
            ));
        }
        std::fs::remove_dir_all(&canon).map_err(|e| TaskError::from_io(&e, "remove plugin"))?;
        let mut s = self.load_state();
        s.plugins.remove(id);
        self.save_state(&s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let host = PluginHost::new(dir.path().to_path_buf());
        let pd = dir.path().join("com.example.namer");
        std::fs::create_dir_all(&pd).unwrap();
        std::fs::write(pd.join("swoop-plugin.json"), r#"{"id":"com.example.namer","name":"Namer","version":"1.0.0","kind":"naming","permissions":["tasks:read"]}"#).unwrap();
        // bad manifest ignored
        let bad = dir.path().join("bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(
            bad.join("swoop-plugin.json"),
            r#"{"id":"bad","name":"B","version":"x","kind":"naming"}"#,
        )
        .unwrap();
        let list = host.scan();
        assert_eq!(list.len(), 1);
        assert!(!list[0].enabled);
        assert!(host.set_enabled("com.example.namer", true, vec![]).is_err());
        let m = host
            .set_enabled(
                "com.example.namer",
                true,
                vec!["tasks:read".into(), "network".into()],
            )
            .unwrap();
        assert!(m.enabled);
        assert_eq!(m.granted_permissions, vec!["tasks:read".to_string()]);
        host.uninstall("com.example.namer").unwrap();
        assert!(host.scan().is_empty());
    }
}
