//! `probe`: everything the Add dialog shows before a task exists.

use crate::api::ProbeResult;
use crate::engine::Engine;
use osprey_domain::{DomainError, DomainResult, NewTaskRequest, TaskKind};
use osprey_runtime::engine::{ResolvedMetadata, Transfer};
use std::sync::Arc;

/// Timeout for magnet metadata resolution in the add dialog.
const MAGNET_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

impl Engine {
    pub(crate) async fn probe_inner(&self, request: NewTaskRequest) -> DomainResult<ProbeResult> {
        let (mut task, outcome) = self.build_task(&request, None).await?;
        let settings = self.settings();
        let secrets = self.resolve_secrets(&task).await;
        let mut warnings = Vec::new();
        let metadata: ResolvedMetadata = match task.kind {
            TaskKind::Torrent => ResolvedMetadata {
                name: Some(task.name.clone()),
                total: task.torrent.as_ref().map(|t| t.total_size),
                torrent: task.torrent.clone(),
                ..Default::default()
            },
            TaskKind::Magnet => {
                let uri = task.source.primary_url().unwrap_or("").to_owned();
                match self
                    .torrent
                    .resolve_magnet(&uri, MAGNET_PROBE_TIMEOUT)
                    .await
                {
                    Ok((info, bytes)) => {
                        let _ = self.store.put_torrent_blob(&info.info_hash, bytes).await;
                        ResolvedMetadata {
                            name: Some(info.name.clone()),
                            total: Some(info.total_size),
                            torrent: Some(info),
                            ..Default::default()
                        }
                    }
                    Err(e) => {
                        warnings.push(format!("magnet metadata unavailable: {}", e.message));
                        ResolvedMetadata::default()
                    }
                }
            }
            TaskKind::Http | TaskKind::Metalink => self
                .http
                .probe(&task, settings.clone(), secrets, self.clients.clone())
                .await
                .map_err(|e| DomainError::Engine(e.message))?,
            TaskKind::Ftp => self
                .ftp
                .probe(&task, settings.clone(), secrets, self.clients.clone())
                .await
                .map_err(|e| DomainError::Engine(e.message))?,
            TaskKind::Hls => self
                .hls
                .probe(&task, settings.clone(), secrets, self.clients.clone())
                .await
                .map_err(|e| DomainError::Engine(e.message))?,
        };
        if let Some(n) = metadata.name.as_deref().filter(|n| !n.trim().is_empty()) {
            if !task.name_locked {
                task.name = osprey_runtime::safety::sanitize_filename(n);
            }
        }
        if let Some(total) = metadata.total {
            task.progress.total = Some(total);
        }
        if let Some(m) = &metadata.mime {
            task.mime = Some(m.split(';').next().unwrap_or("").trim().to_owned());
        }
        if task.category_id.is_none() {
            task.category_id = self.auto_category(&task);
        }
        let dir = task.directory.clone();
        let free = tokio::task::spawn_blocking(move || osprey_runtime::disk::free_space(&dir))
            .await
            .unwrap_or(None);
        if let (Some(total), Some(free)) = (metadata.total, free) {
            if total.saturating_add(settings.storage.reserved_free_space) > free {
                warnings.push("not enough free space on the destination volume".into());
            }
        }
        if let Some(total) = metadata.total {
            if settings.storage.large_download_warning > 0
                && total > settings.storage.large_download_warning
            {
                warnings.push(format!("large download ({} bytes)", total));
            }
        }
        if metadata.resumable == Some(false) {
            warnings
                .push("the server does not support resuming; a pause restarts the download".into());
        }
        if metadata
            .media
            .as_ref()
            .map(|m| m.protected)
            .unwrap_or(false)
        {
            warnings.push("the media is protected (DRM) and cannot be downloaded".into());
        }
        if let Some(mime) = &task.mime {
            if mime.starts_with("text/html") && task.kind == TaskKind::Http {
                warnings.push("the URL returns an HTML page, not a file".into());
            }
        }
        let duplicate = self.detect_duplicate(&task).await;
        Ok(ProbeResult {
            kind: task.kind,
            metadata,
            suggested_name: task.name.clone(),
            suggested_directory: task.directory.clone(),
            suggested_queue: task.queue_id.clone(),
            suggested_category: task.category_id.clone(),
            free_space: free,
            duplicate,
            applicable_rules: outcome.applied,
            warnings,
        })
    }

    pub(crate) async fn detect_media_inner(
        &self,
        url: String,
        page_url: Option<String>,
    ) -> DomainResult<osprey_domain::media::DetectedMedia> {
        osprey_media::detect::detect(&url, page_url, Arc::clone(&self.clients))
            .await
            .map_err(|e| match e.kind {
                osprey_domain::ErrorKind::InvalidUrl
                | osprey_domain::ErrorKind::UnsupportedScheme => DomainError::validation(e.message),
                _ => DomainError::Engine(e.message),
            })
    }
}
