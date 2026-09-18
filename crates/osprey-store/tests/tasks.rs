mod common;

use common::*;
use osprey_domain::events::{LogLevel, TaskLogEntry};
use osprey_domain::state::PauseReason;
use osprey_domain::*;
use osprey_store::{Store, StoreError, TaskFilter, TaskSort};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[tokio::test]
async fn task_round_trip_with_progress_and_segment_map() {
    let store = Store::open_in_memory().unwrap();
    let t = full_task("ubuntu.iso", PathBuf::from("/tmp/dl"));
    store.insert_task(&t).await.unwrap();

    let got = store.get_task(&t.id).await.unwrap().expect("present");
    // transient speed fields are not persisted; everything else must match exactly
    let mut expected = t.clone();
    expected.progress.speed = 0;
    assert_eq!(got, expected);

    // checkpoint was derived from segment_map
    let cp = store
        .get_checkpoint(&t.id)
        .await
        .unwrap()
        .expect("checkpoint");
    assert_eq!(cp, Checkpoint::Segments(t.segment_map.clone().unwrap()));

    let all = store.load_all_tasks().await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0], expected);
    assert_eq!(store.count_tasks(&TaskFilter::default()).await.unwrap(), 1);
}

#[tokio::test]
async fn update_variants_and_delete() {
    let store = Store::open_in_memory().unwrap();
    let mut t = task_in_state("a.bin", PathBuf::from("/tmp"), TaskState::Queued);
    store.insert_task(&t).await.unwrap();

    // cheap state update
    t.transition(TaskState::Connecting).unwrap();
    t.transition(TaskState::Retrying).unwrap();
    t.next_retry_at = Some(Millis(123));
    t.attempt = 2;
    t.error = Some(TaskError::new(ErrorKind::ServerError, "boom"));
    t.block(PauseReason::Shutdown);
    store.update_task_state(&t).await.unwrap();
    let got = store.get_task(&t.id).await.unwrap().unwrap();
    assert_eq!(got.state, TaskState::Retrying);
    assert_eq!(got.rev, t.rev);
    assert_eq!(got.next_retry_at, Some(Millis(123)));
    assert_eq!(got.attempt, 2);
    assert_eq!(
        got.error.as_ref().map(|e| e.kind),
        Some(ErrorKind::ServerError)
    );
    assert_eq!(got.blocked_by, vec![PauseReason::Shutdown]);
    assert_eq!(got.updated_at, t.updated_at);

    // full cold update
    t.name = "renamed.bin".into();
    t.tags = vec!["x".into()];
    t.priority = Priority::Urgent;
    t.touch();
    store.update_task(&t).await.unwrap();
    let got = store.get_task(&t.id).await.unwrap().unwrap();
    assert_eq!(got.name, "renamed.bin");
    assert_eq!(got.tags, vec!["x".to_string()]);
    assert_eq!(got.priority, Priority::Urgent);

    // update of an unknown task is NotFound
    let ghost = task_in_state("ghost", PathBuf::from("/tmp"), TaskState::Pending);
    assert!(matches!(
        store.update_task(&ghost).await,
        Err(StoreError::NotFound(_))
    ));
    assert!(matches!(
        store.update_task_state(&ghost).await,
        Err(StoreError::NotFound(_))
    ));

    // delete cascades to hot rows and log
    store
        .upsert_checkpoint(&t.id, &Checkpoint::Segments(SegmentMap::default()))
        .await
        .unwrap();
    store
        .append_task_log(vec![TaskLogEntry {
            task_id: t.id.clone(),
            at: Millis(1),
            level: LogLevel::Info,
            code: "x".into(),
            message: "m".into(),
        }])
        .await
        .unwrap();
    store.flush().await.unwrap();
    assert!(store.delete_task(&t.id).await.unwrap());
    assert!(!store.delete_task(&t.id).await.unwrap());
    assert!(store.get_task(&t.id).await.unwrap().is_none());
    assert!(store.get_checkpoint(&t.id).await.unwrap().is_none());
    assert!(store.task_log(&t.id, 0).await.unwrap().is_empty());
}

#[tokio::test]
async fn checkpoint_crud_for_every_kind() {
    let store = Store::open_in_memory().unwrap();
    let t = task_in_state("h.ts", PathBuf::from("/tmp"), TaskState::Downloading);
    store.insert_task(&t).await.unwrap();
    assert!(store.get_checkpoint(&t.id).await.unwrap().is_none());

    let mut hls = HlsCheckpoint {
        media_playlist_url: "https://m/x.m3u8".into(),
        part_dir: PathBuf::from("/tmp/h.ts.osprey-part"),
        segment_count: 10,
        ..Default::default()
    };
    hls.set_done(3);
    let cp = Checkpoint::Hls(hls);
    store.upsert_checkpoint(&t.id, &cp).await.unwrap();
    assert_eq!(store.get_checkpoint(&t.id).await.unwrap(), Some(cp));

    let cp = Checkpoint::Torrent(TorrentCheckpoint {
        info_hash: "ab".repeat(20),
        selected_files: Some(vec![1, 2]),
        output_folder: PathBuf::from("/tmp/tor"),
        ..Default::default()
    });
    store.upsert_checkpoint(&t.id, &cp).await.unwrap();
    assert_eq!(store.get_checkpoint(&t.id).await.unwrap(), Some(cp));

    assert!(store.delete_checkpoint(&t.id).await.unwrap());
    assert!(!store.delete_checkpoint(&t.id).await.unwrap());
}

#[tokio::test]
async fn task_log_is_trimmed_to_500_and_ordered() {
    let store = Store::open_in_memory().unwrap();
    let t = task_in_state("l", PathBuf::from("/tmp"), TaskState::Queued);
    store.insert_task(&t).await.unwrap();
    let mk = |i: i64| TaskLogEntry {
        task_id: t.id.clone(),
        at: Millis(i),
        level: if i % 2 == 0 {
            LogLevel::Debug
        } else {
            LogLevel::Warn
        },
        code: format!("code.{i}"),
        message: format!("m{i}"),
    };
    for chunk in (0..700i64).collect::<Vec<_>>().chunks(100) {
        store
            .append_task_log(chunk.iter().map(|i| mk(*i)).collect())
            .await
            .unwrap();
    }
    // a line for an unknown task is dropped, not an error
    store
        .append_task_log(vec![TaskLogEntry {
            task_id: TaskId::new(),
            ..mk(1)
        }])
        .await
        .unwrap();
    store.flush().await.unwrap();
    let all = store.task_log(&t.id, 0).await.unwrap();
    assert_eq!(all.len(), 500);
    assert_eq!(all.first().unwrap().at, Millis(200));
    assert_eq!(all.last().unwrap().at, Millis(699));
    assert_eq!(all[1].level, LogLevel::Warn);
    let last3 = store.task_log(&t.id, 3).await.unwrap();
    assert_eq!(
        last3.iter().map(|e| e.at.0).collect::<Vec<_>>(),
        vec![697, 698, 699]
    );
}

#[tokio::test]
async fn torrent_blobs() {
    let store = Store::open_in_memory().unwrap();
    let hash = "AB".repeat(20);
    assert!(store.get_torrent_blob(&hash).await.unwrap().is_none());
    store
        .put_torrent_blob(&hash, b"d8:announce3:abce".to_vec())
        .await
        .unwrap();
    assert_eq!(
        store.get_torrent_blob(&hash.to_lowercase()).await.unwrap(),
        Some(b"d8:announce3:abce".to_vec())
    );
    store.put_torrent_blob(&hash, vec![1, 2, 3]).await.unwrap();
    assert_eq!(
        store.get_torrent_blob(&hash).await.unwrap(),
        Some(vec![1, 2, 3])
    );
    assert!(store.delete_torrent_blob(&hash).await.unwrap());
    assert!(!store.delete_torrent_blob(&hash).await.unwrap());
}

#[tokio::test]
async fn complete_task_is_atomic() {
    let store = Store::open_in_memory().unwrap();
    let mut t = task_in_state("done.zip", PathBuf::from("/tmp"), TaskState::Verifying);
    t.progress.total = Some(10_000);
    t.segment_map = Some(SegmentMap {
        total: Some(10_000),
        ..Default::default()
    });
    store.insert_task(&t).await.unwrap();
    assert!(store.get_checkpoint(&t.id).await.unwrap().is_some());

    let mut done = t.clone();
    done.transition(TaskState::Completed).unwrap();
    done.file_path = Some(PathBuf::from("/tmp/done.zip"));
    done.verified_checksum = Some(Checksum::new(ChecksumAlgorithm::Sha256, "cd".repeat(32)));
    done.progress.downloaded = 10_000;
    done.segment_map = None;

    // Poisoned history row (empty name violates the CHECK constraint): nothing may change.
    let mut bad = history_for(&done, "done.zip");
    bad.name = String::new();
    let err = store.complete_task(&done, &bad).await.unwrap_err();
    assert!(matches!(err, StoreError::Sqlite(_)), "{err}");
    let still = store.get_task(&t.id).await.unwrap().unwrap();
    assert_eq!(still.state, TaskState::Verifying);
    assert!(still.verified_checksum.is_none());
    assert!(still.segment_map.is_some());
    assert!(store.get_checkpoint(&t.id).await.unwrap().is_some());
    assert_eq!(store.count_history(&Default::default()).await.unwrap(), 0);

    // Now the real thing.
    let good = history_for(&done, "done.zip");
    store.complete_task(&done, &good).await.unwrap();
    let got = store.get_task(&t.id).await.unwrap().unwrap();
    assert_eq!(got.state, TaskState::Completed);
    assert_eq!(got.verified_checksum, done.verified_checksum);
    assert_eq!(got.file_path, done.file_path);
    assert_eq!(got.progress.downloaded, 10_000);
    assert!(got.segment_map.is_none());
    assert!(store.get_checkpoint(&t.id).await.unwrap().is_none());
    let hist = store.query_history(&Default::default()).await.unwrap();
    assert_eq!(hist, vec![good]);
    // the store must not fail if the writer thread had a failed op before
    assert_eq!(store.pending_ops(), 0);
}

#[tokio::test]
async fn list_tasks_filters_and_sorts() {
    let store = Store::open_in_memory().unwrap();
    let dir = PathBuf::from("/tmp");
    let mut a = task_in_state("alpha.iso", dir.clone(), TaskState::Downloading);
    a.progress.total = Some(5_000);
    a.tags = vec!["linux".into()];
    a.position = 3;
    a.created_at = Millis(100);
    let mut b = task_in_state("beta.pdf", dir.clone(), TaskState::Completed);
    b.progress.total = Some(50);
    b.category_id = Some(CategoryId("cat-documents".into()));
    b.position = 1;
    b.created_at = Millis(200);
    let mut c = Task::new(
        TaskKind::Magnet,
        Source::Magnet {
            uri: "magnet:?xt=urn:btih:abc".into(),
        },
        "gamma",
        dir.clone(),
        QueueId("queue-torrent".into()),
    );
    c.position = 2;
    c.created_at = Millis(300);
    c.transition(TaskState::Queued).unwrap();
    c.transition(TaskState::Paused).unwrap();
    let mut d = task_in_state("delta.mp4", dir.clone(), TaskState::Failed);
    d.source = Source::Hls {
        playlist_url: "https://video.example.net/x.m3u8".into(),
        variant: None,
    };
    d.kind = TaskKind::Hls;
    d.position = 4;
    d.created_at = Millis(400);
    for t in [&a, &b, &c, &d] {
        store.insert_task(t).await.unwrap();
    }
    let names = |v: Vec<Task>| v.into_iter().map(|t| t.name).collect::<Vec<_>>();

    let f = TaskFilter::default();
    assert_eq!(
        names(store.list_tasks(&f).await.unwrap()),
        ["beta.pdf", "gamma", "alpha.iso", "delta.mp4"]
    );
    let f = TaskFilter {
        states: vec![TaskState::Downloading, TaskState::Failed],
        sort: TaskSort::CreatedAt,
        descending: true,
        ..Default::default()
    };
    assert_eq!(
        names(store.list_tasks(&f).await.unwrap()),
        ["delta.mp4", "alpha.iso"]
    );
    let f = TaskFilter {
        kinds: vec![TaskKind::Magnet],
        ..Default::default()
    };
    assert_eq!(names(store.list_tasks(&f).await.unwrap()), ["gamma"]);
    let f = TaskFilter {
        queue_id: Some(QueueId("queue-torrent".into())),
        ..Default::default()
    };
    assert_eq!(store.count_tasks(&f).await.unwrap(), 1);
    let f = TaskFilter {
        category_id: Some(CategoryId("cat-documents".into())),
        ..Default::default()
    };
    assert_eq!(names(store.list_tasks(&f).await.unwrap()), ["beta.pdf"]);
    let f = TaskFilter {
        domain: Some("Video.Example.net".into()),
        ..Default::default()
    };
    assert_eq!(names(store.list_tasks(&f).await.unwrap()), ["delta.mp4"]);
    let f = TaskFilter {
        tag: Some("linux".into()),
        ..Default::default()
    };
    assert_eq!(names(store.list_tasks(&f).await.unwrap()), ["alpha.iso"]);
    let f = TaskFilter {
        text: Some("EXAMPLE.NET".into()),
        ..Default::default()
    };
    assert_eq!(names(store.list_tasks(&f).await.unwrap()), ["delta.mp4"]);
    let f = TaskFilter {
        text: Some("%".into()),
        ..Default::default()
    };
    assert!(store.list_tasks(&f).await.unwrap().is_empty());
    let f = TaskFilter {
        created_since: Some(Millis(250)),
        ..Default::default()
    };
    assert_eq!(store.count_tasks(&f).await.unwrap(), 2);
    let f = TaskFilter {
        min_size: Some(100),
        ..Default::default()
    };
    assert_eq!(names(store.list_tasks(&f).await.unwrap()), ["alpha.iso"]);
    let f = TaskFilter {
        max_size: Some(100),
        sort: TaskSort::Size,
        ..Default::default()
    };
    assert_eq!(
        names(store.list_tasks(&f).await.unwrap()),
        ["gamma", "delta.mp4", "beta.pdf"]
    );
    let f = TaskFilter {
        sort: TaskSort::Name,
        limit: 2,
        offset: 1,
        ..Default::default()
    };
    assert_eq!(
        names(store.list_tasks(&f).await.unwrap()),
        ["beta.pdf", "delta.mp4"]
    );
    for (smart, expect) in [
        ("active", vec!["alpha.iso"]),
        ("complete", vec!["beta.pdf"]),
        ("failed", vec!["delta.mp4"]),
        ("paused", vec!["gamma"]),
        ("torrent", vec!["gamma"]),
        ("media", vec!["delta.mp4"]),
        ("queued", vec![]),
        (
            "nonsense",
            vec!["beta.pdf", "gamma", "alpha.iso", "delta.mp4"],
        ),
    ] {
        let f = TaskFilter {
            smart: Some(smart.into()),
            ..Default::default()
        };
        assert_eq!(
            names(store.list_tasks(&f).await.unwrap()),
            expect,
            "smart={smart}"
        );
    }
    let f = TaskFilter {
        sort: TaskSort::Progress,
        descending: true,
        ..Default::default()
    };
    // no downloaded bytes anywhere: falls back to created_at DESC tiebreak
    assert_eq!(store.list_tasks(&f).await.unwrap().len(), 4);
}

#[tokio::test]
async fn load_all_1000_tasks_is_fast() {
    let store = Store::open_in_memory().unwrap();
    let dir = PathBuf::from("/tmp/perf");
    let started = Instant::now();
    for i in 0..1000 {
        let t = full_task(&format!("file-{i}.bin"), dir.clone());
        store.insert_task(&t).await.unwrap();
    }
    let insert_time = started.elapsed();
    let started = Instant::now();
    let all = store.load_all_tasks().await.unwrap();
    let load_time = started.elapsed();
    assert_eq!(all.len(), 1000);
    assert!(all
        .iter()
        .all(|t| t.segment_map.is_some() && t.progress.downloaded == 1234));
    assert!(
        load_time < Duration::from_secs(1),
        "load_all took {load_time:?} (inserts {insert_time:?})"
    );
}

#[tokio::test]
async fn progress_upserts_at_4hz_keep_queue_bounded() {
    let store = Store::open_in_memory().unwrap();
    let dir = PathBuf::from("/tmp");
    let mut ids = Vec::new();
    for i in 0..50 {
        let t = task_in_state(&format!("p{i}"), dir.clone(), TaskState::Downloading);
        store.insert_task(&t).await.unwrap();
        ids.push(t.id);
    }
    let mut max_pending = 0usize;
    let mut ticker = tokio::time::interval(Duration::from_millis(250));
    let start = Instant::now();
    let mut n = 0u64;
    while start.elapsed() < Duration::from_secs(5) {
        ticker.tick().await;
        n += 1;
        for id in &ids {
            let p = Progress {
                downloaded: n * 1000,
                total: Some(1_000_000),
                ..Default::default()
            };
            store.upsert_progress(id, &p).await.unwrap();
        }
        max_pending = max_pending.max(store.pending_ops());
    }
    // one tick's worth (50) plus whatever the writer is mid-way through; never runaway growth
    assert!(max_pending <= 150, "pending ops peaked at {max_pending}");
    store.flush().await.unwrap();
    assert_eq!(store.pending_ops(), 0);
    let got = store.get_task(&ids[0]).await.unwrap().unwrap();
    assert_eq!(got.progress.downloaded, n * 1000);
    assert_eq!(got.progress.total, Some(1_000_000));
}

#[tokio::test]
async fn progress_batcher_writes_latest_value() {
    let store = Store::open_in_memory().unwrap();
    let t = task_in_state("b", PathBuf::from("/tmp"), TaskState::Downloading);
    store.insert_task(&t).await.unwrap();
    let batcher = store.progress_writer();
    for i in 1..=100u64 {
        batcher.update(
            &t.id,
            &Progress {
                downloaded: i,
                ..Default::default()
            },
        );
    }
    assert_eq!(batcher.pending(), 1);
    batcher.flush().await.unwrap();
    assert_eq!(batcher.pending(), 0);
    assert_eq!(
        store
            .get_task(&t.id)
            .await
            .unwrap()
            .unwrap()
            .progress
            .downloaded,
        100
    );
    // the background tick also drains
    batcher.update(
        &t.id,
        &Progress {
            downloaded: 7,
            ..Default::default()
        },
    );
    tokio::time::sleep(Duration::from_millis(2600)).await;
    store.flush().await.unwrap();
    assert_eq!(
        store
            .get_task(&t.id)
            .await
            .unwrap()
            .unwrap()
            .progress
            .downloaded,
        7
    );
}
