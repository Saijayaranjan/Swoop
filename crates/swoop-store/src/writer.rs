//! The single writer thread.
//!
//! One `std::thread` owns the read-write [`rusqlite::Connection`]. Operations arrive over an
//! mpsc channel as boxed closures and are applied in batches: everything that accumulated
//! during a ~250 ms tick is committed in one `BEGIN IMMEDIATE` transaction. A *synchronous*
//! operation (one whose caller awaits an acknowledgement) short-circuits the tick so latency
//! for state transitions stays low, while high-frequency deferred writes (progress, log lines)
//! ride along in the next batch.
//!
//! Each operation runs inside its own `SAVEPOINT`, so a failing operation is rolled back on its
//! own and reported to its caller without discarding the rest of the batch. Acknowledgements are
//! sent only after the batch `COMMIT` succeeded, so a caller that awaited an ack knows the data
//! is durable (to the extent `synchronous=NORMAL` guarantees in WAL mode: durable across
//! application crashes, and across power loss up to the last checkpoint).

use crate::error::{StoreError, StoreResult};
use rusqlite::{Connection, TransactionBehavior};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

/// How long deferred operations may accumulate before they are committed.
pub const BATCH_TICK: Duration = Duration::from_millis(250);
/// After this long without a write the WAL is checkpointed (`TRUNCATE`).
pub const IDLE_CHECKPOINT_AFTER: Duration = Duration::from_secs(30);
/// Polling interval while idle (drives the idle checkpoint).
const IDLE_POLL: Duration = Duration::from_secs(1);
/// Upper bound on operations per transaction (keeps a single huge batch from holding the
/// write lock for too long).
const MAX_BATCH: usize = 2048;

/// A unit of work executed on the writer thread inside a savepoint.
pub(crate) type WriteFn = Box<dyn FnOnce(&Connection) -> StoreResult<()> + Send + 'static>;

/// Acknowledgement channel for synchronous operations.
pub(crate) type Ack = oneshot::Sender<StoreResult<()>>;

/// Control operations that are not plain writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpKind {
    /// Ordinary write closure.
    Write,
    /// No-op barrier: acknowledged once everything queued before it is committed.
    Flush,
    /// Commit what is queued, checkpoint the WAL and stop the thread.
    Shutdown,
}

/// One queued operation.
pub(crate) struct StoreOp {
    /// Short name for diagnostics (`insert_task`, `upsert_progress`, …).
    pub name: &'static str,
    pub kind: OpKind,
    pub run: WriteFn,
    /// `Some` for synchronous operations: the caller awaits the result and the batch is
    /// committed without waiting for the tick. `None` for deferred operations, whose failures
    /// are only logged.
    pub ack: Option<Ack>,
}

impl StoreOp {
    fn is_sync(&self) -> bool {
        self.ack.is_some()
    }
}

/// Handle to the writer thread held by the [`crate::Store`].
pub(crate) struct Writer {
    tx: Sender<StoreOp>,
    pending: Arc<AtomicUsize>,
    thread: Option<JoinHandle<()>>,
}

impl Writer {
    /// Spawn the writer thread owning `conn`.
    pub fn spawn(conn: Connection) -> StoreResult<Self> {
        let (tx, rx) = mpsc::channel::<StoreOp>();
        let pending = Arc::new(AtomicUsize::new(0));
        let pending_for_thread = Arc::clone(&pending);
        let thread = std::thread::Builder::new()
            .name("swoop-store-writer".into())
            .spawn(move || run(conn, rx, pending_for_thread))?;
        Ok(Self {
            tx,
            pending,
            thread: Some(thread),
        })
    }

    /// Enqueue an operation. Fails with [`StoreError::WriterGone`] if the thread has exited.
    pub fn send(&self, op: StoreOp) -> StoreResult<()> {
        self.pending.fetch_add(1, Ordering::SeqCst);
        self.tx.send(op).map_err(|e| {
            self.pending.fetch_sub(1, Ordering::SeqCst);
            if let Some(ack) = e.0.ack {
                let _ = ack.send(Err(StoreError::WriterGone));
            }
            StoreError::WriterGone
        })
    }

    /// Operations enqueued but not yet applied.
    pub fn pending(&self) -> usize {
        self.pending.load(Ordering::SeqCst)
    }

    /// Take the join handle (used once by `close`).
    pub fn take_thread(&mut self) -> Option<JoinHandle<()>> {
        self.thread.take()
    }
}

/// Thread body.
fn run(mut conn: Connection, rx: Receiver<StoreOp>, pending: Arc<AtomicUsize>) {
    let mut last_write = Instant::now();
    let mut idle_checkpointed = true;
    loop {
        // Block until the first operation of the next batch arrives, checkpointing when idle.
        let first = match rx.recv_timeout(IDLE_POLL) {
            Ok(op) => op,
            Err(RecvTimeoutError::Timeout) => {
                if !idle_checkpointed && last_write.elapsed() >= IDLE_CHECKPOINT_AFTER {
                    checkpoint(&conn, "idle");
                    idle_checkpointed = true;
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let mut has_sync = first.is_sync();
        let mut batch = vec![first];
        let mut disconnected = false;
        let deadline = Instant::now() + BATCH_TICK;
        // Gather deferred work until the tick elapses or a synchronous op needs an answer.
        while !has_sync && batch.len() < MAX_BATCH {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            match rx.recv_timeout(deadline - now) {
                Ok(op) => {
                    has_sync = op.is_sync();
                    batch.push(op);
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        // Bundle anything else that is already waiting so it is not delayed by a whole tick.
        while batch.len() < MAX_BATCH {
            match rx.try_recv() {
                Ok(op) => batch.push(op),
                Err(_) => break,
            }
        }
        let shutdown = apply_batch(&mut conn, batch, &pending);
        last_write = Instant::now();
        idle_checkpointed = false;
        if shutdown || disconnected {
            break;
        }
    }
    // Refuse whatever is still queued, then leave the WAL small for the next start.
    while let Ok(op) = rx.try_recv() {
        pending.fetch_sub(1, Ordering::SeqCst);
        if let Some(ack) = op.ack {
            let _ = ack.send(Err(StoreError::Closed));
        }
    }
    checkpoint(&conn, "shutdown");
    tracing::debug!("store writer thread exiting");
}

/// Outcome of one operation, kept until the batch commit decides what to tell the caller.
struct Applied {
    name: &'static str,
    ack: Option<Ack>,
    result: StoreResult<()>,
}

/// Everything a committed (or failed) batch produced.
struct BatchOutcome {
    applied: Vec<Applied>,
    /// `Some` when the batch contained a shutdown op (with its ack channel).
    shutdown: Option<Option<Ack>>,
    /// Ops queued after the shutdown op; refused with `Closed`.
    refused: Vec<StoreOp>,
    /// Error message if `BEGIN` or `COMMIT` failed.
    error: Option<String>,
    failures: usize,
}

/// Run the batch inside one `BEGIN IMMEDIATE`; the transaction borrow ends when this returns.
fn run_transaction(conn: &mut Connection, batch: Vec<StoreOp>) -> BatchOutcome {
    let mut out = BatchOutcome {
        applied: Vec::with_capacity(batch.len()),
        shutdown: None,
        refused: Vec::new(),
        error: None,
        failures: 0,
    };
    let tx = match conn.transaction_with_behavior(TransactionBehavior::Immediate) {
        Ok(tx) => tx,
        Err(e) => {
            out.error = Some(format!("begin transaction failed: {e}"));
            for op in batch {
                if op.kind == OpKind::Shutdown {
                    out.shutdown = Some(op.ack);
                } else {
                    out.applied.push(Applied {
                        name: op.name,
                        ack: op.ack,
                        result: Ok(()),
                    });
                }
            }
            return out;
        }
    };
    for op in batch {
        if out.shutdown.is_some() {
            out.refused.push(op);
            continue;
        }
        let StoreOp {
            name,
            kind,
            run,
            ack,
        } = op;
        match kind {
            OpKind::Shutdown => out.shutdown = Some(ack),
            OpKind::Flush => out.applied.push(Applied {
                name,
                ack,
                result: Ok(()),
            }),
            OpKind::Write => {
                let result = run_in_savepoint(&tx, run);
                if result.is_err() {
                    out.failures += 1;
                }
                out.applied.push(Applied { name, ack, result });
            }
        }
    }
    if let Err(e) = tx.commit() {
        out.error = Some(format!("commit failed: {e}"));
    }
    out
}

/// Apply one batch in a single transaction, deliver results, and checkpoint on shutdown.
/// Returns `true` when a [`OpKind::Shutdown`] was processed.
fn apply_batch(conn: &mut Connection, batch: Vec<StoreOp>, pending: &AtomicUsize) -> bool {
    let started = Instant::now();
    let count = batch.len();
    let outcome = run_transaction(conn, batch);
    if let Some(err) = &outcome.error {
        tracing::error!(error = %err, ops = count, "store batch failed");
    }
    for Applied { name, ack, result } in outcome.applied {
        pending.fetch_sub(1, Ordering::SeqCst);
        let result = match &outcome.error {
            Some(err) => Err(StoreError::Internal(err.clone())),
            None => result,
        };
        match (ack, result) {
            (Some(ack), r) => {
                let _ = ack.send(r);
            }
            (None, Err(e)) => tracing::warn!(op = name, error = %e, "deferred store op failed"),
            (None, Ok(())) => {}
        }
    }
    tracing::trace!(
        ops = count,
        failures = outcome.failures,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "store batch applied"
    );
    let Some(ack) = outcome.shutdown else {
        return false;
    };
    for op in outcome.refused {
        pending.fetch_sub(1, Ordering::SeqCst);
        if let Some(ack) = op.ack {
            let _ = ack.send(Err(StoreError::Closed));
        }
    }
    checkpoint(conn, "shutdown");
    pending.fetch_sub(1, Ordering::SeqCst);
    if let Some(ack) = ack {
        let _ = ack.send(match outcome.error {
            Some(err) => Err(StoreError::Internal(err)),
            None => Ok(()),
        });
    }
    true
}

/// Run one closure inside a savepoint so its failure does not poison the batch.
fn run_in_savepoint(conn: &Connection, run: WriteFn) -> StoreResult<()> {
    conn.execute_batch("SAVEPOINT op")?;
    match run(conn) {
        Ok(()) => {
            conn.execute_batch("RELEASE op")?;
            Ok(())
        }
        Err(e) => {
            if let Err(rb) = conn.execute_batch("ROLLBACK TO op; RELEASE op") {
                tracing::error!(error = %rb, "savepoint rollback failed");
            }
            Err(e)
        }
    }
}

/// `PRAGMA wal_checkpoint(TRUNCATE)`; failures are logged, never fatal.
fn checkpoint(conn: &Connection, reason: &str) {
    match conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
        ))
    }) {
        Ok((busy, log, checkpointed)) => {
            tracing::debug!(reason, busy, log, checkpointed, "wal checkpoint")
        }
        Err(e) => tracing::warn!(reason, error = %e, "wal checkpoint failed"),
    }
}
