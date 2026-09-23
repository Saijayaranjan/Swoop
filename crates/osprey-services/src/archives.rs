//! Archive inspection and extraction (adapter over `osprey-archive`).

use crate::api::{ArchiveEntry, ArchiveListing};
use crate::engine::Engine;
use osprey_domain::{DomainError, DomainResult};
use std::path::PathBuf;

/// Entries returned by a listing before it is truncated.
pub const LIST_LIMIT: usize = 10_000;

impl From<osprey_archive::ArchiveEntry> for ArchiveEntry {
    fn from(e: osprey_archive::ArchiveEntry) -> Self {
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

impl From<osprey_archive::ArchiveListing> for ArchiveListing {
    fn from(l: osprey_archive::ArchiveListing) -> Self {
        Self {
            format: l.format,
            entries: l.entries.into_iter().map(ArchiveEntry::from).collect(),
            truncated: l.truncated,
            intact: l.intact,
            supports_selective_extraction: l.supports_selective_extraction,
        }
    }
}

fn map_err(e: osprey_domain::TaskError) -> DomainError {
    match e.kind {
        osprey_domain::ErrorKind::NotFound | osprey_domain::ErrorKind::VolumeUnavailable => {
            DomainError::NotFound(e.message)
        }
        osprey_domain::ErrorKind::PathTraversal
        | osprey_domain::ErrorKind::InvalidFilename
        | osprey_domain::ErrorKind::ParseError
        | osprey_domain::ErrorKind::UnexpectedContent => DomainError::Validation(e.message),
        osprey_domain::ErrorKind::PermissionDenied => DomainError::PermissionDenied(e.message),
        _ => DomainError::Engine(e.message),
    }
}

impl Engine {
    pub(crate) async fn archive_list_inner(&self, path: PathBuf) -> DomainResult<ArchiveListing> {
        let path = Self::expand(&path);
        if !path.is_file() {
            return Err(DomainError::not_found(path.display()));
        }
        osprey_archive::list(&path, LIST_LIMIT)
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
        osprey_runtime::safety::validate_destination_dir(&destination)
            .map_err(|e| DomainError::validation(e.message))?;
        osprey_archive::extract(&path, entries.as_deref(), &destination)
            .await
            .map_err(map_err)
    }
}
