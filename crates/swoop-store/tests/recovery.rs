mod common;

use common::*;
use std::path::{Path, PathBuf};
use swoop_domain::state::PauseReason;
use swoop_domain::*;
use swoop_store::Store;

const SUFFIX: &str = ".swoop-part";

fn part(dir: &Path, name: &str) {
    std::fs::write(dir.join(format!("{name}{SUFFIX}")), b"partial").unwrap();
}

fn part_dir(dir: &Path, name: &str) {
    std::fs::create_dir_all(dir.join(format!("{name}{SUFFIX}"))).unwrap();
}

fn final_file(dir: &Path, name: &str, size: usize) {
    std::fs::write(dir.join(name), vec![0u8; size]).unwrap();
}

/// One task per row of the spec table; returns `(label, task)`.
async fn seed(store: &Store, dir: &Path) -> Vec<(&'static str, Task)> {
    let mut rows: Vec<(&'static str, Task)> = Vec::new();
    let mk = |name: &str, state: TaskState| task_in_state(name, dir.to_path_buf(), state);

    rows.push(("pending", mk("pending", TaskState::Pending)));
    let mut q = mk("queued", TaskState::Queued);
    q.block(PauseReason::Shutdown);
    rows.push(("queued", q));
    let mut s = mk("scheduled", TaskState::Scheduled);
    s.block(PauseReason::Schedule("night".into()));
    rows.push(("scheduled", s));

    for (label, state) in [
        ("resolving_present", TaskState::Resolving),
        ("connecting_present", TaskState::Connecting),
        ("downloading_present", TaskState::Downloading),
        ("retrying_present", TaskState::Retrying),
    ] {
        let mut t = mk(label, state);
        t.block(PauseReason::Shutdown);
        t.progress.downloaded = 500;
        t.progress.total = Some(1_000);
        t.segment_map = Some(SegmentMap {
            segments: vec![Segment::new(0, 0, 1_000)],
            total: Some(1_000),
            part_path: Some(dir.join(format!("{label}{SUFFIX}"))),
            ..Default::default()
        });
        part(dir, label);
        rows.push((label, t));
    }
    for (label, state) in [
        ("resolving_missing", TaskState::Resolving),
        ("downloading_missing", TaskState::Downloading),
        ("retrying_missing", TaskState::Retrying),
    ] {
        let mut t = mk(label, state);
        t.progress.downloaded = 500;
        t.progress.total = Some(1_000);
        t.segment_map = Some(SegmentMap {
            segments: vec![Segment::new(0, 0, 1_000)],
            total: Some(1_000),
            ..Default::default()
        });
        rows.push((label, t));
    }

    let mut p = mk("paused", TaskState::Paused);
    p.block(PauseReason::User);
    p.block(PauseReason::Shutdown);
    p.segment_map = Some(SegmentMap::default());
    rows.push(("paused", p));

    let mut v = mk("verifying_present", TaskState::Verifying);
    v.segment_map = Some(SegmentMap::default());
    part(dir, "verifying_present");
    rows.push(("verifying_present", v));

    let mut vc = mk("verifying_complete", TaskState::Verifying);
    vc.progress.total = Some(64);
    final_file(dir, "verifying_complete", 64);
    rows.push(("verifying_complete", vc));

    let mut pc = mk("processing_complete", TaskState::Processing);
    pc.progress.total = Some(32);
    final_file(dir, "processing_complete", 32);
    rows.push(("processing_complete", pc));

    let mut vw = mk("verifying_wrong_size", TaskState::Verifying);
    vw.progress.total = Some(64);
    final_file(dir, "verifying_wrong_size", 10);
    rows.push(("verifying_wrong_size", vw));

    let mut ph = mk("processing_hls", TaskState::Processing);
    ph.kind = TaskKind::Hls;
    part_dir(dir, "processing_hls");
    rows.push(("processing_hls", ph));

    for (label, state) in [
        ("completed", TaskState::Completed),
        ("failed", TaskState::Failed),
        ("cancelled", TaskState::Cancelled),
    ] {
        rows.push((label, mk(label, state)));
    }

    let mut seed_t = mk("seeding", TaskState::Seeding);
    seed_t.kind = TaskKind::Torrent;
    seed_t.source = Source::TorrentFile {
        info_hash: "ab".repeat(20),
        name: "seeding".into(),
    };
    rows.push(("seeding", seed_t));

    let mut tor = mk("torrent_downloading", TaskState::Downloading);
    tor.kind = TaskKind::Magnet;
    tor.source = Source::Magnet {
        uri: "magnet:?xt=urn:btih:cd".into(),
    };
    rows.push(("torrent_downloading", tor));

    for (_, t) in &rows {
        store.insert_task(t).await.unwrap();
    }
    // HLS checkpoint pointing at the part dir
    let hls_id = rows
        .iter()
        .find(|(l, _)| *l == "processing_hls")
        .map(|(_, t)| t.id.clone())
        .unwrap();
    store
        .upsert_checkpoint(
            &hls_id,
            &Checkpoint::Hls(HlsCheckpoint {
                part_dir: dir.join(format!("processing_hls{SUFFIX}")),
                segment_count: 3,
                ..Default::default()
            }),
        )
        .await
        .unwrap();
    rows
}

#[tokio::test]
async fn recovery_table_is_applied_exactly() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("downloads");
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open_in_memory().unwrap();
    let rows = seed(&store, &dir).await;
    let id = |label: &str| {
        rows.iter()
            .find(|(l, _)| *l == label)
            .map(|(_, t)| t.id.clone())
            .unwrap()
    };

    let report = store.recover(SUFFIX).await.unwrap();
    assert_eq!(report.scanned, rows.len());
    assert!(report.failed.is_empty(), "{:?}", report.failed);

    let mut requeued_cp = report.requeued_with_checkpoint.clone();
    requeued_cp.sort();
    let mut expect = vec![
        id("resolving_present"),
        id("connecting_present"),
        id("downloading_present"),
        id("retrying_present"),
        id("torrent_downloading"),
    ];
    expect.sort();
    assert_eq!(requeued_cp, expect);

    let mut scratch = report.requeued_from_scratch.clone();
    scratch.sort();
    let mut expect = vec![
        id("resolving_missing"),
        id("downloading_missing"),
        id("retrying_missing"),
        id("verifying_wrong_size"),
    ];
    expect.sort();
    assert_eq!(scratch, expect);

    assert_eq!(report.reverifying, vec![id("verifying_present")]);
    assert_eq!(report.reprocessing, vec![id("processing_hls")]);
    let mut ready = report.ready_to_complete.clone();
    ready.sort();
    let mut expect = vec![id("verifying_complete"), id("processing_complete")];
    expect.sort();
    assert_eq!(ready, expect);
    assert_eq!(report.reseeding, vec![id("seeding")]);
    let mut readd = report.torrents_to_readd.clone();
    readd.sort();
    let mut expect = vec![id("seeding"), id("torrent_downloading")];
    expect.sort();
    assert_eq!(readd, expect);
    assert_eq!(report.paused_by_shutdown, vec![id("paused")]);
    // kept: pending, queued, scheduled, paused, completed, failed, cancelled, verifying_complete,
    // processing_complete (ready_to_complete is not a change either)
    assert_eq!(report.kept, 7);
    assert_eq!(report.tasks.len(), rows.len());

    // Persisted states match the table.
    let state_of = |label: &str| {
        let i = id(label);
        let store = store.clone();
        async move { store.get_task(&i).await.unwrap().unwrap() }
    };
    assert_eq!(state_of("pending").await.state, TaskState::Pending);
    let q = state_of("queued").await;
    assert_eq!(q.state, TaskState::Queued);
    assert_eq!(q.blocked_by, vec![PauseReason::Shutdown], "kept untouched");
    let s = state_of("scheduled").await;
    assert_eq!(s.state, TaskState::Scheduled);
    assert_eq!(s.blocked_by, vec![PauseReason::Schedule("night".into())]);

    for label in [
        "resolving_present",
        "connecting_present",
        "downloading_present",
        "retrying_present",
    ] {
        let t = state_of(label).await;
        assert_eq!(t.state, TaskState::Queued, "{label}");
        assert!(t.blocked_by.is_empty(), "{label}: Shutdown cleared");
        assert!(t.segment_map.is_some(), "{label}: checkpoint kept");
        assert_eq!(t.progress.downloaded, 500, "{label}: progress kept");
        assert!(t.next_retry_at.is_none());
    }
    for label in [
        "resolving_missing",
        "downloading_missing",
        "retrying_missing",
    ] {
        let t = state_of(label).await;
        assert_eq!(t.state, TaskState::Queued, "{label}");
        assert!(t.segment_map.is_none(), "{label}: checkpoint discarded");
        assert!(store.get_checkpoint(&t.id).await.unwrap().is_none());
        assert_eq!(t.progress.downloaded, 0, "{label}: progress reset");
        assert_eq!(t.progress.total, Some(1_000), "{label}: known size kept");
    }
    let p = state_of("paused").await;
    assert_eq!(p.state, TaskState::Paused);
    assert_eq!(p.blocked_by, vec![PauseReason::User, PauseReason::Shutdown]);
    assert!(p.segment_map.is_some());

    let v = state_of("verifying_present").await;
    assert_eq!(v.state, TaskState::Verifying);
    assert!(v.segment_map.is_some());
    assert_eq!(
        state_of("verifying_complete").await.state,
        TaskState::Verifying
    );
    assert_eq!(
        state_of("processing_complete").await.state,
        TaskState::Processing
    );
    let vw = state_of("verifying_wrong_size").await;
    assert_eq!(vw.state, TaskState::Queued);
    assert_eq!(
        state_of("processing_hls").await.state,
        TaskState::Processing
    );
    assert!(store
        .get_checkpoint(&id("processing_hls"))
        .await
        .unwrap()
        .is_some());

    assert_eq!(state_of("completed").await.state, TaskState::Completed);
    assert_eq!(state_of("failed").await.state, TaskState::Failed);
    assert_eq!(state_of("cancelled").await.state, TaskState::Cancelled);
    let seeding = state_of("seeding").await;
    assert_eq!(seeding.state, TaskState::Resolving);
    assert!(seeding.blocked_by.is_empty());
    assert_eq!(
        state_of("torrent_downloading").await.state,
        TaskState::Queued
    );

    // The report snapshot equals what is persisted.
    let mut persisted = store.load_all_tasks().await.unwrap();
    let mut snapshot = report.tasks.clone();
    persisted.sort_by(|a, b| a.id.cmp(&b.id));
    snapshot.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(persisted, snapshot);

    // A second run is stable: only the reseeded torrent (now `Resolving`, an active state)
    // moves again, everything else is untouched.
    let again = store.recover(SUFFIX).await.unwrap();
    assert_eq!(again.requeued_with_checkpoint, vec![id("seeding")]);
    assert!(again.requeued_from_scratch.is_empty());
    assert!(again.reseeding.is_empty());
    assert_eq!(again.ready_to_complete.len(), 2);
    assert_eq!(again.reverifying.len(), 1);
    assert_eq!(again.reprocessing.len(), 1);
    assert_eq!(again.tasks.len(), rows.len());
    let mut after = store.load_all_tasks().await.unwrap();
    after.sort_by(|a, b| a.id.cmp(&b.id));
    after.retain(|t| t.id != id("seeding"));
    persisted.retain(|t| t.id != id("seeding"));
    assert_eq!(after, persisted);
}

#[tokio::test]
async fn recovery_uses_checkpoint_part_path() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    // part file lives somewhere other than <dir>/<name><suffix>
    let elsewhere = tmp.path().join("elsewhere.part");
    std::fs::write(&elsewhere, b"x").unwrap();
    let mut t = task_in_state("moved", PathBuf::from(tmp.path()), TaskState::Downloading);
    t.segment_map = Some(SegmentMap {
        part_path: Some(elsewhere),
        ..Default::default()
    });
    store.insert_task(&t).await.unwrap();
    let report = store.recover(SUFFIX).await.unwrap();
    assert_eq!(report.requeued_with_checkpoint, vec![t.id.clone()]);
    assert!(store.get_checkpoint(&t.id).await.unwrap().is_some());
}
