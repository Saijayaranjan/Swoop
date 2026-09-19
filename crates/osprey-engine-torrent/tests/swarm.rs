//! End-to-end tests against a second librqbit session on loopback. DHT, LSD and (unless a
//! test says otherwise) trackers are disabled; peers are injected directly.

mod common;

use common::*;
use osprey_domain::checkpoint::Checkpoint;
use osprey_domain::{ErrorKind, TaskState};
use osprey_engine_torrent::TorrentEngine;
use osprey_runtime::engine::{FileSelectionUpdate, Transfer, TransferControl, TransferOutcome};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

const FILES: &[(&str, usize)] = &[
    ("one.bin", 1_500_000),
    ("nested/two.bin", 1_200_000),
    ("three.bin", 4 * 1024 * 1024 - 2_700_000),
];

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloads_from_local_seed_to_completion() {
    let seed = seeder(FILES, Vec::new()).await;
    let base = TempDir::with_prefix("osprey-b").unwrap();
    let settings = settings(|_| {});
    let engine = engine(
        base.path(),
        settings.clone(),
        blobs(&[(&seed.info_hash, &seed.bytes)]),
        vec![seed.addr],
        false,
    );
    let out = base.path().join("downloads");
    std::fs::create_dir_all(&out).unwrap();
    let task = torrent_task(&seed, out.clone());
    let id = task_id(&task);
    let sink = Arc::new(RecordingSink::default());
    let control = TransferControl::new();
    let outcome = tokio::time::timeout(
        Duration::from_secs(120),
        engine.run(context(task, settings, control, sink.clone(), None)),
    )
    .await
    .expect("run timed out");

    let root = out.join("payload");
    assert_eq!(
        outcome,
        TransferOutcome::Completed {
            file_path: root.clone(),
            bytes: 4 * 1024 * 1024
        }
    );
    assert_files_identical(&seed, &root);

    let states = sink.states();
    assert!(states.contains(&TaskState::Connecting), "{states:?}");
    assert!(states.contains(&TaskState::Seeding), "{states:?}");
    {
        let rec = sink.0.lock();
        let meta = rec.metadata.last().expect("metadata emitted");
        let info = meta.torrent.as_ref().expect("torrent info");
        assert!(info.have_metadata);
        assert_eq!(info.files.len(), 3);
        assert_eq!(info.total_size, 4 * 1024 * 1024);
        assert_eq!(meta.file_path.as_deref(), Some(root.as_path()));
        assert_eq!(meta.name.as_deref(), Some("payload"));
        let last = rec.progress.last().expect("progress emitted");
        assert_eq!(last.downloaded, 4 * 1024 * 1024);
        assert!(rec
            .checkpoints
            .iter()
            .any(|c| matches!(c, Checkpoint::Torrent(_))));
    }

    let live = engine.torrent_info(&id).expect("torrent info after run");
    assert_eq!(live.piece_count, 64);
    assert_eq!(live.piece_map_rle, vec![0, 64], "all pieces present");
    assert!((live.availability - 1.0).abs() < f32::EPSILON);
    assert!(live.files.iter().all(|f| f.downloaded == f.size));
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_protocol_then_resume_from_checkpoint() {
    let seed = seeder(FILES, Vec::new()).await;
    let base = TempDir::with_prefix("osprey-b").unwrap();
    let settings = settings(|_| {});
    let engine = engine(
        base.path(),
        settings.clone(),
        blobs(&[(&seed.info_hash, &seed.bytes)]),
        vec![seed.addr],
        false,
    );
    let out = base.path().join("downloads");
    std::fs::create_dir_all(&out).unwrap();
    let task = torrent_task(&seed, out.clone());
    let sink = Arc::new(RecordingSink::default());
    let control = TransferControl::new();
    // Throttle so the download is still in flight when we pause (4 MiB at 400 KiB/s ≈ 10 s).
    control.download_limit.store(400 * 1024, Ordering::Relaxed);

    let run = {
        let engine = engine.clone();
        let ctx = context(
            task.clone(),
            settings.clone(),
            control.clone(),
            sink.clone(),
            None,
        );
        tokio::spawn(async move { engine.run(ctx).await })
    };
    let started = wait_for(Duration::from_secs(60), || {
        sink.last_progress()
            .is_some_and(|p| p.downloaded > 0 && p.downloaded < 4 * 1024 * 1024)
    })
    .await;
    assert!(
        started,
        "download never made partial progress: {:?}",
        sink.last_progress()
    );

    let t0 = std::time::Instant::now();
    control.pause.cancel();
    let outcome = tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .expect("pause hung")
        .unwrap();
    assert_eq!(outcome, TransferOutcome::Paused);
    assert!(
        t0.elapsed() < Duration::from_secs(2),
        "paused after {:?}",
        t0.elapsed()
    );
    let cps = sink.torrent_checkpoints();
    let cp = cps.last().expect("checkpoint on pause");
    assert_eq!(cp.info_hash, seed.info_hash);
    assert_eq!(cp.output_folder, out.join("payload"));
    assert!(cp.selected_files.is_none());

    // Resume with the checkpoint and a fresh control (limit lifted → librqbit re-add path).
    let sink2 = Arc::new(RecordingSink::default());
    let control2 = TransferControl::new();
    let outcome = tokio::time::timeout(
        Duration::from_secs(120),
        engine.run(context(
            task,
            settings,
            control2,
            sink2.clone(),
            Some(Checkpoint::Torrent(cp.clone())),
        )),
    )
    .await
    .expect("resume timed out");
    assert!(
        matches!(outcome, TransferOutcome::Completed { .. }),
        "{outcome:?}"
    );
    assert_files_identical(&seed, &out.join("payload"));
    let first = sink2
        .0
        .lock()
        .progress
        .first()
        .cloned()
        .expect("progress after resume");
    assert!(
        first.downloaded >= cp.downloaded.saturating_sub(16 * 1024 * 1024),
        "resume lost verified data: first={} checkpoint={}",
        first.downloaded,
        cp.downloaded
    );
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn file_selection_downloads_only_selected_files() {
    let seed = seeder(FILES, Vec::new()).await;
    let base = TempDir::with_prefix("osprey-b").unwrap();
    let settings = settings(|_| {});
    let engine = engine(
        base.path(),
        settings.clone(),
        blobs(&[(&seed.info_hash, &seed.bytes)]),
        vec![seed.addr],
        false,
    );
    let out = base.path().join("downloads");
    std::fs::create_dir_all(&out).unwrap();
    let task = torrent_task(&seed, out.clone());
    let id = task_id(&task);
    let sink = Arc::new(RecordingSink::default());
    let control = TransferControl::new();
    // librqbit numbers files in directory-walk order; keep only `one.bin`.
    let files = TorrentEngine::parse_torrent(&seed.bytes).unwrap().files;
    let keep = files.iter().find(|f| f.path == "one.bin").unwrap().index;
    let dropped: Vec<u32> = files
        .iter()
        .map(|f| f.index)
        .filter(|i| *i != keep)
        .collect();
    control.set_file_selection(
        dropped
            .iter()
            .enumerate()
            .map(|(n, i)| FileSelectionUpdate {
                index: *i,
                selected: false,
                priority: if n == 0 { 1 } else { 0 },
            })
            .collect(),
    );
    let outcome = tokio::time::timeout(
        Duration::from_secs(120),
        engine.run(context(task, settings, control, sink.clone(), None)),
    )
    .await
    .expect("run timed out");
    let root = out.join("payload");
    assert!(
        matches!(outcome, TransferOutcome::Completed { .. }),
        "{outcome:?}"
    );

    for (name, data) in &seed.files {
        let on_disk = std::fs::read(root.join(name)).unwrap_or_default();
        if name == "one.bin" {
            assert!(on_disk == *data, "one.bin must be complete");
        } else {
            assert!(on_disk != *data, "{name} should not have been downloaded");
        }
    }
    let cp = sink.torrent_checkpoints().pop().expect("checkpoint");
    assert_eq!(cp.selected_files, Some(vec![keep]));
    assert_eq!(cp.priorities.get(&dropped[1]), Some(&0));
    let info = engine.torrent_info(&id).unwrap();
    for f in &info.files {
        assert_eq!(f.selected, f.index == keep, "{f:?}");
    }
    // totals are piece-granular: the selected file rounded up to whole 64 KiB pieces
    let total = sink.last_progress().unwrap().total.unwrap();
    assert!(
        (1_500_000..1_500_000 + 2 * 65_536).contains(&total),
        "{total}"
    );
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn seeding_continues_until_stopped_when_ratio_is_unlimited() {
    let seed = seeder(&[("solo.bin", 600_000)], Vec::new()).await;
    let base = TempDir::with_prefix("osprey-b").unwrap();
    let settings = settings(|s| s.torrent.seed_ratio_limit = -1.0);
    let engine = engine(
        base.path(),
        settings.clone(),
        blobs(&[(&seed.info_hash, &seed.bytes)]),
        vec![seed.addr],
        false,
    );
    let out = base.path().join("downloads");
    std::fs::create_dir_all(&out).unwrap();
    let task = torrent_task(&seed, out.clone());
    let id = task_id(&task);
    let sink = Arc::new(RecordingSink::default());
    let control = TransferControl::new();
    let run = {
        let engine = engine.clone();
        let ctx = context(task, settings, control.clone(), sink.clone(), None);
        tokio::spawn(async move { engine.run(ctx).await })
    };
    assert!(
        wait_for(Duration::from_secs(60), || sink
            .states()
            .contains(&TaskState::Seeding))
        .await,
        "never reached Seeding: {:?}",
        sink.states()
    );
    // Still running while seeding is allowed…
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!run.is_finished());
    let info = engine.torrent_info(&id).unwrap();
    assert!(info.seeding_since.is_some());
    // …until a limit applies live.
    engine.set_seeding_limits(
        &id,
        osprey_domain::torrent::SeedingLimits {
            ratio_limit: Some(0.0),
            ..Default::default()
        },
    );
    let outcome = tokio::time::timeout(Duration::from_secs(10), run)
        .await
        .expect("did not stop")
        .unwrap();
    assert!(
        matches!(outcome, TransferOutcome::Completed { .. }),
        "{outcome:?}"
    );
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn magnet_resolves_through_peer_and_downloads() {
    let seed = seeder(&[("m1.bin", 700_000), ("m2.bin", 300_000)], Vec::new()).await;
    let base = TempDir::with_prefix("osprey-b").unwrap();
    let settings = settings(|_| {});
    let engine = engine(
        base.path(),
        settings.clone(),
        blobs(&[]),
        vec![seed.addr],
        false,
    );
    let uri = format!("magnet:?xt=urn:btih:{}&dn=payload", seed.info_hash);

    // Add-dialog path: list only, returns metainfo bytes we could store as a blob.
    let (info, bytes) = engine
        .resolve_magnet(&uri, Duration::from_secs(60))
        .await
        .expect("resolve_magnet");
    assert!(info.have_metadata);
    assert_eq!(info.files.len(), 2);
    assert_eq!(
        TorrentEngine::parse_torrent(&bytes).unwrap().info_hash,
        seed.info_hash
    );

    let out = base.path().join("downloads");
    std::fs::create_dir_all(&out).unwrap();
    let task = magnet_task(&uri, out.clone());
    let sink = Arc::new(RecordingSink::default());
    let outcome = tokio::time::timeout(
        Duration::from_secs(120),
        engine.run(context(
            task,
            settings,
            TransferControl::new(),
            sink.clone(),
            None,
        )),
    )
    .await
    .expect("run timed out");
    assert!(
        matches!(outcome, TransferOutcome::Completed { .. }),
        "{outcome:?}"
    );
    assert_files_identical(&seed, &out.join("payload"));
    let states = sink.states();
    assert_eq!(states.first(), Some(&TaskState::Resolving), "{states:?}");
    let metas = sink.0.lock().metadata.clone();
    assert!(!metas[0].torrent.as_ref().unwrap().have_metadata);
    assert!(
        metas
            .last()
            .unwrap()
            .torrent
            .as_ref()
            .unwrap()
            .have_metadata
    );
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn magnet_resolution_times_out_cleanly() {
    let base = TempDir::with_prefix("osprey-b").unwrap();
    let settings = settings(|_| {});
    // Trackers enabled so librqbit has a (dead) peer source and keeps waiting.
    let engine = engine(base.path(), settings, blobs(&[]), Vec::new(), true);
    let uri = "magnet:?xt=urn:btih:00000000000000000000000000000000deadbeef&tr=http%3A%2F%2F127.0.0.1%3A1%2Fannounce";
    let t0 = std::time::Instant::now();
    let err = engine
        .resolve_magnet(uri, Duration::from_secs(3))
        .await
        .expect_err("bogus hash must not resolve");
    assert_eq!(err.kind, ErrorKind::NoPeers, "{err}");
    assert!(
        t0.elapsed() < Duration::from_secs(6),
        "took {:?}",
        t0.elapsed()
    );
    assert!(err.message.contains("3s"));
    // Without any peer source librqbit fails immediately with a classified error.
    let engine2 = common::engine(
        base.path().join("2").as_path(),
        common::settings(|_| {}),
        blobs(&[]),
        Vec::new(),
        false,
    );
    let err = engine2
        .resolve_magnet(
            "magnet:?xt=urn:btih:00000000000000000000000000000000deadbeef",
            Duration::from_secs(3),
        )
        .await
        .expect_err("no peer source");
    assert_eq!(err.kind, ErrorKind::DhtUnavailable, "{err}");
    engine.shutdown().await;
    engine2.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejects_torrents_with_unsafe_paths_and_missing_blobs() {
    let base = TempDir::with_prefix("osprey-b").unwrap();
    let settings = settings(|_| {});
    let evil = evil_torrent();
    let hash = TorrentEngine::parse_torrent(&evil).err().map(|e| e.kind);
    assert_eq!(hash, Some(ErrorKind::InvalidTorrent));

    let engine = engine(
        base.path(),
        settings.clone(),
        blobs(&[("evil", &evil)]),
        Vec::new(),
        false,
    );
    let out = base.path().join("downloads");
    std::fs::create_dir_all(&out).unwrap();
    let mut task = magnet_task(
        "magnet:?xt=urn:btih:00000000000000000000000000000000deadbeef",
        out.clone(),
    );
    task.kind = osprey_domain::TaskKind::Torrent;
    task.source = osprey_domain::Source::TorrentFile {
        info_hash: "evil".into(),
        name: "evil".into(),
    };
    let sink = Arc::new(RecordingSink::default());
    let outcome = engine
        .run(context(
            task.clone(),
            settings.clone(),
            TransferControl::new(),
            sink.clone(),
            None,
        ))
        .await;
    match outcome {
        TransferOutcome::Failed(e) => assert_eq!(e.kind, ErrorKind::InvalidTorrent, "{e}"),
        other => panic!("expected failure, got {other:?}"),
    }
    assert!(!out.join("..").join("escaped").exists());

    task.source = osprey_domain::Source::TorrentFile {
        info_hash: "missing".into(),
        name: "missing".into(),
    };
    match engine
        .run(context(task, settings, TransferControl::new(), sink, None))
        .await
    {
        TransferOutcome::Failed(e) => assert_eq!(e.kind, ErrorKind::InvalidTorrent),
        other => panic!("expected failure, got {other:?}"),
    }
    engine.shutdown().await;
}

/// A syntactically valid torrent whose file path climbs out of the output folder.
fn evil_torrent() -> Vec<u8> {
    // multi-file with a ".." component
    let mut v = Vec::new();
    v.extend_from_slice(b"d4:infod5:filesld6:lengthi10e4:pathl2:..7:escapedeed6:lengthi10e4:pathl4:fineeee4:name4:evil12:piece lengthi16384e6:pieces20:");
    v.extend_from_slice(&[0x41; 20]);
    v.extend_from_slice(b"ee");
    v
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_limit_change_readds_without_losing_pieces() {
    let seed = seeder(FILES, Vec::new()).await;
    let base = TempDir::with_prefix("osprey-b").unwrap();
    let settings = settings(|_| {});
    let engine = engine(
        base.path(),
        settings.clone(),
        blobs(&[(&seed.info_hash, &seed.bytes)]),
        vec![seed.addr],
        false,
    );
    let out = base.path().join("downloads");
    std::fs::create_dir_all(&out).unwrap();
    let task = torrent_task(&seed, out.clone());
    let sink = Arc::new(RecordingSink::default());
    let control = TransferControl::new();
    control.download_limit.store(300 * 1024, Ordering::Relaxed);
    let run = {
        let engine = engine.clone();
        let ctx = context(task, settings, control.clone(), sink.clone(), None);
        tokio::spawn(async move { engine.run(ctx).await })
    };
    assert!(
        wait_for(Duration::from_secs(60), || sink
            .last_progress()
            .is_some_and(|p| p.downloaded > 600_000))
        .await,
        "no partial progress"
    );
    // Lifting the per-torrent limit is an add-time option in librqbit → re-add (debounced 2 s).
    control.download_limit.store(0, Ordering::Relaxed);
    let outcome = tokio::time::timeout(Duration::from_secs(120), run)
        .await
        .expect("timed out")
        .unwrap();
    assert!(
        matches!(outcome, TransferOutcome::Completed { .. }),
        "{outcome:?}"
    );
    assert_files_identical(&seed, &out.join("payload"));
    {
        let rec = sink.0.lock();
        assert!(
            rec.logs.iter().any(|(_, code, _)| code == "torrent.readd"),
            "{:?}",
            rec.logs
        );
        let downloaded: Vec<u64> = rec.progress.iter().map(|p| p.downloaded).collect();
        assert!(
            downloaded.windows(2).all(|w| w[1] >= w[0]),
            "progress went backwards across the re-add: {downloaded:?}"
        );
    }
    engine.shutdown().await;
}
