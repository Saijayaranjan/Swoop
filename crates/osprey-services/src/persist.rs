//! Ordered persistence queue.
//!
//! Engine sinks run on hot paths and cannot await the store; API methods want their writes
//! acknowledged. Both go through one FIFO worker so a task's rows are always written in the
//! order the mutations happened (a later, stale write can never overwrite a newer state).

use osprey_domain::history::HistoryEntry;
use osprey_domain::{Checkpoint, DomainError, DomainResult, Task, TaskId, TaskLogEntry};
use osprey_store::{Store, StoreResult};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

/// One durable write.
#[derive(Debug)]
pub enum PersistOp {
    /// Insert (or replace) a task row.
    Insert(Box<Task>),
    /// Rewrite the full cold row.
    Update(Box<Task>),
    /// Cheap state-only update.
    State(Box<Task>),
    /// Delete a task with its progress, checkpoint and log.
    Delete(TaskId),
    /// Upsert a resume checkpoint.
    Checkpoint(TaskId, Box<Checkpoint>),
    /// Drop a checkpoint (fresh restart).
    DeleteCheckpoint(TaskId),
    /// The completion transaction.
    Complete(Box<Task>, Box<HistoryEntry>),
    /// Append log lines.
    Log(Vec<TaskLogEntry>),
    /// Wait until everything queued before is committed.
    Flush,
}

type Ack = oneshot::Sender<StoreResult<()>>;

/// Handle to the persistence worker. Cheap to clone.
#[derive(Clone)]
pub struct Persister {
    tx: mpsc::UnboundedSender<(PersistOp, Option<Ack>)>,
}

impl Persister {
    /// Spawn the worker on the current runtime.
    pub fn spawn(store: Arc<Store>) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<(PersistOp, Option<Ack>)>();
        tokio::spawn(async move {
            while let Some((op, ack)) = rx.recv().await {
                let name = op_name(&op);
                let result = apply(&store, op).await;
                if let Err(e) = &result {
                    tracing::warn!(op = name, error = %e, "persist failed");
                }
                if let Some(ack) = ack {
                    let _ = ack.send(result);
                }
            }
        });
        Self { tx }
    }

    /// Queue without waiting (sink hot paths). Failures are logged by the worker.
    pub fn send(&self, op: PersistOp) {
        if self.tx.send((op, None)).is_err() {
            tracing::debug!("persist worker gone; dropping write");
        }
    }

    /// Queue and wait for the commit.
    pub async fn commit(&self, op: PersistOp) -> DomainResult<()> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send((op, Some(tx)))
            .map_err(|_| DomainError::Storage("persistence worker stopped".into()))?;
        match rx.await {
            Ok(r) => r.map_err(DomainError::from),
            Err(_) => Err(DomainError::Storage("persistence worker dropped".into())),
        }
    }

    /// Wait until every write queued so far has been committed.
    pub async fn flush(&self) -> DomainResult<()> {
        self.commit(PersistOp::Flush).await
    }
}

fn op_name(op: &PersistOp) -> &'static str {
    match op {
        PersistOp::Insert(_) => "insert",
        PersistOp::Update(_) => "update",
        PersistOp::State(_) => "state",
        PersistOp::Delete(_) => "delete",
        PersistOp::Checkpoint(..) => "checkpoint",
        PersistOp::DeleteCheckpoint(_) => "delete_checkpoint",
        PersistOp::Complete(..) => "complete",
        PersistOp::Log(_) => "log",
        PersistOp::Flush => "flush",
    }
}

async fn apply(store: &Store, op: PersistOp) -> StoreResult<()> {
    match op {
        PersistOp::Insert(t) => store.insert_task(&t).await,
        PersistOp::Update(t) => store.update_task(&t).await,
        PersistOp::State(t) => store.update_task_state(&t).await,
        PersistOp::Delete(id) => store.delete_task(&id).await.map(|_| ()),
        PersistOp::Checkpoint(id, cp) => store.upsert_checkpoint(&id, &cp).await,
        PersistOp::DeleteCheckpoint(id) => store.delete_checkpoint(&id).await.map(|_| ()),
        PersistOp::Complete(t, h) => store.complete_task(&t, &h).await,
        PersistOp::Log(lines) => store.append_task_log(lines).await,
        PersistOp::Flush => store.flush().await,
    }
}
