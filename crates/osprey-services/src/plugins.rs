//! Plugin management (adapter over `osprey-plugins`).

use crate::api::PluginInfo;
use crate::engine::Engine;
use osprey_domain::{DomainError, DomainResult, PluginId};
use osprey_plugins::PluginManifest;

fn to_info(m: PluginManifest) -> PluginInfo {
    PluginInfo {
        id: PluginId(m.id),
        name: m.name,
        version: m.version,
        description: m.description,
        author: m.author,
        kind: m.kind,
        permissions: m.permissions,
        enabled: m.enabled,
        path: m.path,
        granted_permissions: m.granted_permissions,
    }
}

fn map_err(e: osprey_domain::TaskError) -> DomainError {
    match e.kind {
        osprey_domain::ErrorKind::NotFound => DomainError::NotFound(e.message),
        osprey_domain::ErrorKind::PermissionDenied => DomainError::PermissionDenied(e.message),
        osprey_domain::ErrorKind::ParseError | osprey_domain::ErrorKind::InvalidFilename => {
            DomainError::Validation(e.message)
        }
        _ => DomainError::Engine(e.message),
    }
}

impl Engine {
    pub(crate) async fn list_plugins_inner(&self) -> DomainResult<Vec<PluginInfo>> {
        let this = self.this.clone();
        let list = tokio::task::spawn_blocking(move || {
            this.upgrade().map(|e| e.plugins.scan()).unwrap_or_default()
        })
        .await
        .map_err(DomainError::internal)?;
        Ok(list.into_iter().map(to_info).collect())
    }

    pub(crate) async fn set_plugin_enabled_inner(
        &self,
        id: PluginId,
        enabled: bool,
        granted: Vec<String>,
    ) -> DomainResult<PluginInfo> {
        let this = self.this.clone();
        let m = tokio::task::spawn_blocking(move || {
            let e = this
                .upgrade()
                .ok_or_else(|| DomainError::Unavailable("engine stopped".into()))?;
            e.plugins
                .set_enabled(id.as_str(), enabled, granted)
                .map_err(map_err)
        })
        .await
        .map_err(DomainError::internal)??;
        Ok(to_info(m))
    }

    pub(crate) async fn uninstall_plugin_inner(&self, id: PluginId) -> DomainResult<()> {
        let this = self.this.clone();
        tokio::task::spawn_blocking(move || {
            let e = this
                .upgrade()
                .ok_or_else(|| DomainError::Unavailable("engine stopped".into()))?;
            e.plugins.uninstall(id.as_str()).map_err(map_err)
        })
        .await
        .map_err(DomainError::internal)?
    }
}
