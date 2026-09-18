//! Osprey persistence: SQLite (WAL) with a single writer thread, versioned migrations and
//! crash recovery. See `docs/architecture/002-persistence-and-recovery.md` for the spec this
//! crate implements and `README.md` in this crate for the schema/threading overview.
//!
//! # Threading model
//!
//! * **One writer.** A dedicated `std::thread` owns the read-write connection. Every mutation
//!   is a closure sent over an mpsc channel; the thread commits batches in one
//!   `BEGIN IMMEDIATE` per ~250 ms tick, or immediately when a synchronous caller is waiting.
//! * **One reader.** A second, read-only connection serves queries on Tokio's blocking pool.
//!   WAL lets it run concurrently with the writer. Synchronous writes are visible to the reader
//!   as soon as the awaited call returns; deferred writes (progress, log lines, speed samples)
//!   become visible within a tick — call [`Store::flush`] when you need them now.
//!
//! # Synchronous vs deferred
//!
//! Methods documented as *deferred* return as soon as the operation is queued; their failures
//! are logged, not returned. Everything else awaits the commit and returns its result.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod config;
mod devices;
mod error;
mod history;
mod migrations;
mod progress;
mod reader;
mod recovery;
mod settings;
mod tasks;
mod writer;

pub use config::{RecipeRecord, AUTOMATION_RUNS_KEEP};
pub use devices::AUDIT_KEEP;
pub use error::{StoreError, StoreResult};
pub use migrations::CURRENT_SCHEMA_VERSION;
pub use progress::{ProgressBatcher, PROGRESS_FLUSH_INTERVAL};
pub use recovery::{RecoveryAction, RecoveryReport};
pub use settings::{CredentialMeta, SpeedSample, SPEED_SAMPLE_RETENTION_MS};
pub use tasks::{TaskFilter, TaskSort, TASK_LOG_KEEP};
pub use writer::{BATCH_TICK, IDLE_CHECKPOINT_AFTER};

use crate::reader::ReaderConn;
use crate::writer::{OpKind, StoreOp, Writer};
use parking_lot::Mutex;
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::oneshot;

/// Handle to the database. Cheap to share through `Arc`; all methods take `&self`.
pub struct Store {
    path: PathBuf,
    writer: Mutex<Option<Writer>>,
    reader: ReaderConn,
    /// Keeps the backing directory of [`Store::open_in_memory`] alive until the store drops.
    _temp: Option<tempfile::TempDir>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("path", &self.path)
            .field("pending_ops", &self.pending_ops())
            .finish()
    }
}

impl Store {
    /// Open (or create) the database at `path`, apply pragmas and migrations, and start the
    /// writer thread. Blocks briefly (migrations run inline); call it during startup.
    ///
    /// The single-instance lock (`osprey.lock`) is the caller's responsibility and must be
    /// held before this is called.
    pub fn open(path: &Path) -> StoreResult<Arc<Store>> {
        Self::open_inner(path, None)
    }

    /// Open a private, throw-away database for tests. It is backed by a temp directory (not
    /// SQLite's `:memory:` mode) so WAL, the reader connection and checkpoints behave exactly
    /// as in production; the files are removed when the store is dropped.
    pub fn open_in_memory() -> StoreResult<Arc<Store>> {
        let dir = tempfile::Builder::new().prefix("osprey-store-").tempdir()?;
        let path = dir.path().join("osprey.db");
        Self::open_inner(&path, Some(dir))
    }

    fn open_inner(path: &Path, temp: Option<tempfile::TempDir>) -> StoreResult<Arc<Store>> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        apply_writer_pragmas(&conn)?;
        let version = migrations::apply(&mut conn)?;
        tracing::info!(path = %path.display(), schema = version, "store opened");
        let reader = reader::open_reader(path)?;
        let writer = Writer::spawn(conn)?;
        Ok(Arc::new(Store {
            path: path.to_path_buf(),
            writer: Mutex::new(Some(writer)),
            reader: Arc::new(Mutex::new(Some(reader))),
            _temp: temp,
        }))
    }

    /// Location of the database file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Number of write operations queued but not yet committed. Useful to verify that
    /// producers are not outrunning the writer.
    pub fn pending_ops(&self) -> usize {
        self.writer
            .lock()
            .as_ref()
            .map(Writer::pending)
            .unwrap_or(0)
    }

    /// Wait until every operation queued before this call has been committed.
    pub async fn flush(&self) -> StoreResult<()> {
        let (tx, rx) = oneshot::channel();
        self.enqueue(StoreOp {
            name: "flush",
            kind: OpKind::Flush,
            run: Box::new(|_| Ok(())),
            ack: Some(tx),
        })?;
        rx.await.map_err(|_| StoreError::WriterGone)?
    }

    /// Flush, checkpoint the WAL (`TRUNCATE`), stop the writer thread and close both
    /// connections. Every later call fails with [`StoreError::Closed`]. Idempotent.
    pub async fn close(&self) -> StoreResult<()> {
        let Some(mut writer) = self.writer.lock().take() else {
            return Ok(());
        };
        let (tx, rx) = oneshot::channel();
        let send_result = writer.send(StoreOp {
            name: "shutdown",
            kind: OpKind::Shutdown,
            run: Box::new(|_| Ok(())),
            ack: Some(tx),
        });
        let ack = match send_result {
            Ok(()) => rx.await.map_err(|_| StoreError::WriterGone).and_then(|r| r),
            Err(e) => Err(e),
        };
        let thread = writer.take_thread();
        drop(writer); // drops the sender so the thread exits even if the shutdown op was lost
        if let Some(handle) = thread {
            tokio::task::spawn_blocking(move || handle.join())
                .await
                .map_err(|e| StoreError::Internal(format!("join failed: {e}")))?
                .map_err(|_| StoreError::WriterGone)?;
        }
        // Close the reader so SQLite can remove the -wal/-shm files.
        let reader = self.reader.lock().take();
        drop(reader);
        tracing::info!(path = %self.path.display(), "store closed");
        ack
    }

    /// Read a value from the `meta` key/value table.
    pub async fn meta_get(&self, key: &str) -> StoreResult<Option<String>> {
        let key = key.to_owned();
        self.read(move |conn| {
            let mut stmt = conn.prepare_cached("SELECT value FROM meta WHERE key = ?1")?;
            let mut rows = stmt.query([&key])?;
            match rows.next()? {
                Some(row) => Ok(Some(row.get(0)?)),
                None => Ok(None),
            }
        })
        .await
    }

    /// Insert or replace a `meta` value.
    pub async fn meta_set(&self, key: &str, value: &str) -> StoreResult<()> {
        let (key, value) = (key.to_owned(), value.to_owned());
        self.write("meta_set", move |conn| {
            conn.execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [&key, &value],
            )?;
            Ok(())
        })
        .await
    }

    /// Current `PRAGMA user_version` of the open database.
    pub async fn schema_version(&self) -> StoreResult<u32> {
        self.read(migrations::schema_version).await
    }

    // -----------------------------------------------------------------------------------------
    // plumbing shared by the entity modules
    // -----------------------------------------------------------------------------------------

    fn enqueue(&self, op: StoreOp) -> StoreResult<()> {
        match self.writer.lock().as_ref() {
            Some(w) => w.send(op),
            None => {
                if let Some(ack) = op.ack {
                    let _ = ack.send(Err(StoreError::Closed));
                }
                Err(StoreError::Closed)
            }
        }
    }

    /// Run `f` on the writer thread and await the committed result.
    pub(crate) async fn write<T, F>(&self, name: &'static str, f: F) -> StoreResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> StoreResult<T> + Send + 'static,
    {
        let slot: Arc<Mutex<Option<T>>> = Arc::new(Mutex::new(None));
        let sink = Arc::clone(&slot);
        let (tx, rx) = oneshot::channel();
        self.enqueue(StoreOp {
            name,
            kind: OpKind::Write,
            run: Box::new(move |conn| {
                let v = f(conn)?;
                *sink.lock() = Some(v);
                Ok(())
            }),
            ack: Some(tx),
        })?;
        rx.await.map_err(|_| StoreError::WriterGone)??;
        let value = slot.lock().take();
        value.ok_or_else(|| StoreError::Internal(format!("{name}: no result produced")))
    }

    /// Queue `f` for the next batch without waiting. Failures are logged by the writer.
    pub(crate) fn write_deferred<F>(&self, name: &'static str, f: F) -> StoreResult<()>
    where
        F: FnOnce(&Connection) -> StoreResult<()> + Send + 'static,
    {
        self.enqueue(StoreOp {
            name,
            kind: OpKind::Write,
            run: Box::new(f),
            ack: None,
        })
    }

    /// Run `f` against the read-only connection on the blocking pool.
    pub(crate) async fn read<T, F>(&self, f: F) -> StoreResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> StoreResult<T> + Send + 'static,
    {
        reader::read(&self.reader, f).await
    }
}

/// Pragmas from the persistence spec, applied to the writer connection.
fn apply_writer_pragmas(conn: &Connection) -> StoreResult<()> {
    conn.execute_batch(
        "PRAGMA busy_timeout = 5000;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA wal_autocheckpoint = 1000;
         PRAGMA mmap_size = 67108864;
         PRAGMA temp_store = MEMORY;
         PRAGMA cache_size = -16000;",
    )?;
    let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        tracing::warn!(journal_mode = %mode, "WAL mode unavailable; falling back");
    }
    Ok(())
}
