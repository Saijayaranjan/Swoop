mod common;

use common::*;
use osprey_domain::queue::Queue;
use osprey_domain::*;
use osprey_store::{Store, StoreError};
use std::path::PathBuf;

#[tokio::test]
async fn reopen_after_close_keeps_data_and_truncates_wal() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("nested").join("osprey.db");
    let t = full_task("persist.bin", PathBuf::from("/dl"));
    let queue = Queue::new("Q", 2);
    {
        let store = Store::open(&db).unwrap();
        assert_eq!(store.path(), db.as_path());
        store.insert_task(&t).await.unwrap();
        store.upsert_queue(&queue).await.unwrap();
        store.meta_set("k", "v").await.unwrap();
        // generate WAL traffic
        for i in 0..200 {
            store
                .upsert_progress(
                    &t.id,
                    &Progress {
                        downloaded: i,
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
        }
        store.flush().await.unwrap();
        let wal = db.with_extension("db-wal");
        assert!(wal.exists());
        store.close().await.unwrap();
        // idempotent
        store.close().await.unwrap();
        let wal_len = std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);
        assert_eq!(wal_len, 0, "WAL not truncated on close");
        // everything fails cleanly after close
        assert!(matches!(
            store.get_task(&t.id).await,
            Err(StoreError::Closed)
        ));
        assert!(matches!(
            store.meta_set("a", "b").await,
            Err(StoreError::Closed)
        ));
        assert!(matches!(store.flush().await, Err(StoreError::Closed)));
        assert_eq!(store.pending_ops(), 0);
    }
    let store = Store::open(&db).unwrap();
    let got = store.get_task(&t.id).await.unwrap().unwrap();
    assert_eq!(got.progress.downloaded, 199);
    assert_eq!(got.name, "persist.bin");
    assert_eq!(got.segment_map, t.segment_map);
    assert_eq!(store.list_queues().await.unwrap(), vec![queue]);
    assert_eq!(store.meta_get("k").await.unwrap().as_deref(), Some("v"));
    assert_eq!(
        store.schema_version().await.unwrap(),
        osprey_store::CURRENT_SCHEMA_VERSION
    );
    store.close().await.unwrap();
}

#[tokio::test]
async fn store_error_converts_to_domain_error() {
    let e: DomainError = StoreError::NotFound("task x".into()).into();
    assert_eq!(e, DomainError::NotFound("task x".into()));
    let e: DomainError = StoreError::Closed.into();
    assert!(matches!(e, DomainError::Storage(_)));
}

#[tokio::test]
async fn failed_op_does_not_poison_the_batch() {
    let store = Store::open_in_memory().unwrap();
    let t = task_in_state("a", PathBuf::from("/tmp"), TaskState::Queued);
    let ghost = task_in_state("ghost", PathBuf::from("/tmp"), TaskState::Queued);
    // three ops enqueued back to back; the middle one fails
    let f1 = store.insert_task(&t);
    let f2 = store.update_task(&ghost);
    let f3 = store.meta_set("after", "ok");
    let (r1, r2, r3) = tokio::join!(f1, f2, f3);
    r1.unwrap();
    assert!(matches!(r2, Err(StoreError::NotFound(_))));
    r3.unwrap();
    assert!(store.get_task(&t.id).await.unwrap().is_some());
    assert_eq!(
        store.meta_get("after").await.unwrap().as_deref(),
        Some("ok")
    );
}

#[tokio::test]
async fn debug_and_drop_without_close() {
    let store = Store::open_in_memory().unwrap();
    assert!(format!("{store:?}").contains("pending_ops"));
    store.meta_set("x", "y").await.unwrap();
    drop(store); // writer thread exits on its own when the sender is dropped
}
