//! Store error type. Everything the persistence layer can fail with is classified here; the
//! services layer converts it into [`osprey_domain::DomainError::Storage`].

use osprey_domain::DomainError;

/// Errors raised by the store.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// SQLite reported an error (I/O, constraint, busy, …).
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// A JSON column could not be (de)serialised.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// Filesystem error while opening the database or inspecting part files.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// A migration failed; the transaction was rolled back and the schema is unchanged.
    #[error("migration to schema v{version} failed: {source}")]
    Migration {
        /// Target schema version of the failed migration.
        version: u32,
        /// Underlying SQLite error.
        #[source]
        source: rusqlite::Error,
    },
    /// The database was written by a newer Osprey and cannot be opened safely.
    #[error("database schema v{found} is newer than the supported v{supported}")]
    SchemaTooNew {
        /// `PRAGMA user_version` found on disk.
        found: u32,
        /// Highest version this build knows.
        supported: u32,
    },
    /// The store has been closed; no further operations are accepted.
    #[error("store is closed")]
    Closed,
    /// The writer thread exited unexpectedly (a panic in an operation).
    #[error("store writer thread is gone")]
    WriterGone,
    /// The requested row does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// A row holds data that no longer parses (schema drift, manual edits).
    #[error("corrupt row: {0}")]
    Corrupt(String),
    /// A domain invariant (e.g. an illegal state transition during recovery) was violated.
    #[error("domain: {0}")]
    Domain(#[from] DomainError),
    /// Internal invariant broken; always a bug.
    #[error("internal store error: {0}")]
    Internal(String),
}

impl From<StoreError> for DomainError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::NotFound(what) => DomainError::NotFound(what),
            StoreError::Domain(d) => d,
            other => DomainError::Storage(other.to_string()),
        }
    }
}

/// Convenience alias used throughout the crate.
pub type StoreResult<T> = Result<T, StoreError>;
