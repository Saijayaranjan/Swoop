//! Tracker probes and peer discovery against in-process HTTP (axum) and UDP tracker stubs.

mod common;

use common::*;
use osprey_domain::ErrorKind;
use osprey_runtime::engine::{Transfer, TransferControl, TransferOutcome};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn discovers_peers_through_trackers_and_reports_health() {
    // The seeder is only reachable through the trackers: no initial peers are injected.
    let seed = seeder(&[("t1.bin", 900_000), ("t2.bin", 300_000)], Vec::new()).await;
    let http = http_stub(vec![seed.addr], 5, 2, None).await;
    let udp = udp_stub(vec![seed.addr], 7, 1).await;
    let dead = "http://127.0.0.1:1/announce".to_string();
    let udp_url = format!("udp://{}/announce", udp.addr);

    // Re-announce the seeder's torrent under our tracker set.
    let parsed = osprey_engine_torrent::metainfo::parse_torrent(&seed.bytes).unwrap();
    let bytes = osprey_engine_torrent::metainfo::wrap_with_trackers(
        &parsed.info_bytes,
        &[
            vec![http.url.clone()],
            vec![udp_url.clone()],
            vec![dead.clone()],
        ],
        None,
        None,
    );

    let base = TempDir::with_prefix("osprey-t").unwrap();
    let settings = settings(|s| {
        s.torrent.seed_ratio_limit = -1.0;
        s.torrent.announce_interval_seconds = 2;
    });
    let engine = engine(
        base.path(),
        settings.clone(),
        blobs(&[(&seed.info_hash, &bytes)]),
        Vec::new(),
        true,
    );
    let out = base.path().join("downloads");
    std::fs::create_dir_all(&out).unwrap();
    let task = torrent_task(&seed, out.clone());
    let id = task_id(&task);
    let sink = Arc::new(RecordingSink::default());
    let control = TransferControl::new();
    // Throttle so the peer list can be observed while the transfer is in flight.
    control.download_limit.store(200 * 1024, Ordering::Relaxed);
    let run = {
        let engine = engine.clone();
        let ctx = context(task, settings, control.clone(), sink.clone(), None);
        tokio::spawn(async move { engine.run(ctx).await })
    };

    let saw_peer = wait_for(Duration::from_secs(60), || {
        engine
            .peers(&id)
            .iter()
            .any(|p| p.address == seed.addr.to_string())
    })
    .await;
    assert!(
        saw_peer,
        "seeder never showed up in the peer list: {:?}",
        engine.peers(&id)
    );
    let peers = engine.peers(&id);
    let seed_peer = peers
        .iter()
        .find(|p| p.address == seed.addr.to_string())
        .unwrap();
    assert!(seed_peer.flags.contains('T'), "{seed_peer:?}");

    let done = wait_for(Duration::from_secs(90), || {
        sink.last_progress()
            .is_some_and(|p| p.downloaded == 1_200_000)
    })
    .await;
    assert!(
        done,
        "download via tracker-discovered peer did not finish: {:?}",
        sink.last_progress()
    );
    assert_files_identical(&seed, &out.join("payload"));

    let healthy = wait_for(Duration::from_secs(30), || {
        let t = engine.trackers(&id);
        t.iter().any(|s| s.url == http.url && s.health == "working")
            && t.iter().any(|s| s.url == udp_url && s.health == "working")
    })
    .await;
    let trackers = engine.trackers(&id);
    assert!(healthy, "{trackers:?}");
    let h = trackers.iter().find(|t| t.url == http.url).unwrap();
    assert_eq!(h.seeders, Some(5));
    assert_eq!(h.leechers, Some(2));
    assert_eq!(h.tier, 0);
    assert!(h.latency_ms.is_some());
    assert!(h.last_announce_at.is_some() && h.next_announce_at.is_some());
    let u = trackers.iter().find(|t| t.url == udp_url).unwrap();
    assert_eq!(u.seeders, Some(7));
    assert_eq!(u.tier, 1);
    assert!(udp.announces.load(Ordering::Relaxed) >= 1);

    // Our probe announced with numwant=0 and the same peer id/port librqbit uses; the passkey
    // in the tracker URL survived.
    let queries = http.queries.lock().clone();
    assert!(queries
        .iter()
        .any(|q| q.contains("numwant=0") && q.starts_with("passkey=SECRET123&info_hash=")));
    assert!(
        queries.iter().any(|q| !q.contains("numwant=0")),
        "librqbit's own announce also arrived"
    );

    // The unreachable tracker degrades to error and then dead after three failures.
    let dead_now = wait_for(Duration::from_secs(40), || {
        engine
            .trackers(&id)
            .iter()
            .any(|s| s.url == dead && s.health == "dead")
    })
    .await;
    let d = engine
        .trackers(&id)
        .into_iter()
        .find(|s| s.url == dead)
        .unwrap();
    assert!(dead_now, "{d:?}");
    assert!(d.consecutive_failures >= 3);
    assert!(d.last_error.is_some());
    // Passkeys never reach logs.
    assert!(sink
        .0
        .lock()
        .logs
        .iter()
        .all(|(_, _, m)| !m.contains("SECRET123")));

    // Tracker management on a loaded task.
    engine.set_tracker_enabled(&id, &dead, false).unwrap();
    assert_eq!(
        engine
            .trackers(&id)
            .iter()
            .find(|s| s.url == dead)
            .map(|s| s.health.as_str()),
        Some("disabled")
    );
    assert!(engine
        .add_trackers(&id, vec!["wss://ws.example/announce".into()])
        .is_err());
    assert_eq!(
        engine
            .add_trackers(&id, vec!["udp://127.0.0.1:9/announce".into()])
            .unwrap(),
        1
    );
    engine.reannounce(&id).unwrap();
    engine
        .remove_tracker(&id, "udp://127.0.0.1:9/announce")
        .unwrap();
    assert!(engine
        .remove_tracker(&id, "udp://127.0.0.1:9/announce")
        .is_err());
    let info = engine.torrent_info(&id).unwrap();
    assert_eq!(info.seeders_total, 7);
    assert_eq!(info.trackers.len(), 3);

    control.cancel.cancel();
    let outcome = tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outcome, TransferOutcome::Cancelled);
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tracker_failure_reason_is_reported_and_private_torrents_refuse_additions() {
    let seed = seeder(&[("p.bin", 200_000)], Vec::new()).await;
    let http = http_stub(Vec::new(), 0, 0, Some("unregistered torrent")).await;
    let parsed = osprey_engine_torrent::metainfo::parse_torrent(&seed.bytes).unwrap();
    let bytes = osprey_engine_torrent::metainfo::wrap_with_trackers(
        &parsed.info_bytes,
        &[vec![http.url.clone()]],
        None,
        None,
    );

    let base = TempDir::with_prefix("osprey-t").unwrap();
    let settings =
        settings(|s| s.torrent.additional_trackers = vec!["http://127.0.0.1:1/extra".into()]);
    let engine = engine(
        base.path(),
        settings.clone(),
        blobs(&[(&seed.info_hash, &bytes)]),
        vec![seed.addr],
        true,
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
    let errored = wait_for(Duration::from_secs(30), || {
        engine.trackers(&id).iter().any(|s| {
            s.url == http.url
                && s.health == "error"
                && s.last_error
                    .as_deref()
                    .is_some_and(|e| e.contains("unregistered"))
        })
    })
    .await;
    assert!(errored, "{:?}", engine.trackers(&id));
    // settings.additional_trackers reached the (public) torrent as its own tier
    assert!(engine
        .trackers(&id)
        .iter()
        .any(|s| s.url == "http://127.0.0.1:1/extra" && s.tier == 1));
    // tracker failures are never fatal: the download still completes through the injected peer
    let outcome = tokio::time::timeout(Duration::from_secs(60), run)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(outcome, TransferOutcome::Completed { .. }),
        "{outcome:?}"
    );

    // Private torrent: no settings trackers, no user additions, DHT/PEX never enabled.
    let private_bytes = private_torrent();
    let pinfo = osprey_engine_torrent::TorrentEngine::parse_torrent(&private_bytes).unwrap();
    assert!(pinfo.private);
    let blobs2 = blobs(&[("private", &private_bytes)]);
    let engine2 = common::engine(
        &base.path().join("2"),
        common::settings(|s| {
            s.torrent.additional_trackers = vec!["http://127.0.0.1:1/extra".into()]
        }),
        blobs2,
        Vec::new(),
        true,
    );
    let mut ptask = torrent_task(&seed, out.clone());
    ptask.source = osprey_domain::Source::TorrentFile {
        info_hash: "private".into(),
        name: "private".into(),
    };
    let pid = task_id(&ptask);
    let psink = Arc::new(RecordingSink::default());
    let pcontrol = TransferControl::new();
    let prun = {
        let engine2 = engine2.clone();
        let ctx = context(
            ptask,
            common::settings(|_| {}),
            pcontrol.clone(),
            psink.clone(),
            None,
        );
        tokio::spawn(async move { engine2.run(ctx).await })
    };
    assert!(
        wait_for(Duration::from_secs(30), || !engine2
            .trackers(&pid)
            .is_empty())
        .await
    );
    let t = engine2.trackers(&pid);
    assert_eq!(t.len(), 1, "{t:?}");
    assert!(t[0].url.starts_with("https://private.example/"));
    let err = engine2
        .add_trackers(&pid, vec!["http://127.0.0.1:1/x".into()])
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::PermissionDenied);
    pcontrol.cancel.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), prun).await;
    engine.shutdown().await;
    engine2.shutdown().await;
}

fn private_torrent() -> Vec<u8> {
    let url = "https://private.example/announce?passkey=k";
    let mut v = Vec::new();
    v.extend_from_slice(
        format!("d8:announce{}:{url}4:infod6:lengthi10e4:name7:private12:piece lengthi16384e6:pieces20:", url.len()).as_bytes(),
    );
    v.extend_from_slice(&[0x42; 20]);
    v.extend_from_slice(b"7:privatei1eee");
    v
}
