//! Archive inspection and extraction (adapter over `swoop-archive`).

use crate::api::{ArchiveEntry, ArchiveListing};
use crate::engine::Engine;
use std::path::PathBuf;
use swoop_domain::{DomainError, DomainResult};

/// Entries returned by a listing before it is truncated.
pub const LIST_LIMIT: usize = 10_000;

impl From<swoop_archive::ArchiveEntry> for ArchiveEntry {
    fn from(e: swoop_archive::ArchiveEntry) -> Self {
        Self {
            path: e.path,
            size: e.size,
            compressed_size: e.compressed_size,
            is_dir: e.is_dir,
            modified: e.modified,
            crc32: e.crc32,
        }
    }
}

impl From<swoop_archive::ArchiveListing> for ArchiveListing {
    fn from(l: swoop_archive::ArchiveListing) -> Self {
        Self {
            format: l.format,
            entries: l.entries.into_iter().map(ArchiveEntry::from).collect(),
            truncated: l.truncated,
            intact: l.intact,
            supports_selective_extraction: l.supports_selective_extraction,
        }
    }
}

fn map_err(e: swoop_domain::TaskError) -> DomainError {
    match e.kind {
        swoop_domain::ErrorKind::NotFound | swoop_domain::ErrorKind::VolumeUnavailable => {
            DomainError::NotFound(e.message)
        }
        swoop_domain::ErrorKind::PathTraversal
        | swoop_domain::ErrorKind::InvalidFilename
        | swoop_domain::ErrorKind::ParseError
        | swoop_domain::ErrorKind::UnexpectedContent => DomainError::Validation(e.message),
        swoop_domain::ErrorKind::PermissionDenied => DomainError::PermissionDenied(e.message),
        _ => DomainError::Engine(e.message),
    }
}

impl Engine {
    pub(crate) async fn archive_list_inner(&self, path: PathBuf) -> DomainResult<ArchiveListing> {
        let path = Self::expand(&path);
        if !path.is_file() {
            return Err(DomainError::not_found(path.display()));
        }
        swoop_archive::list(&path, LIST_LIMIT)
            .await
            .map(ArchiveListing::from)
            .map_err(map_err)
    }

    pub(crate) async fn archive_extract_inner(
        &self,
        path: PathBuf,
        entries: Option<Vec<String>>,
        destination: PathBuf,
    ) -> DomainResult<u32> {
        let path = Self::expand(&path);
        let destination = Self::expand(&destination);
        if !path.is_file() {
            return Err(DomainError::not_found(path.display()));
        }
        swoop_runtime::safety::validate_destination_dir(&destination)
            .map_err(|e| DomainError::validation(e.message))?;
        swoop_archive::extract(&path, entries.as_deref(), &destination)
            .await
            .map_err(map_err)
    }
}
