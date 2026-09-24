//! Engine-level smoke test through the FFI surface (runs on a foreign-style executor: a separate
//! current-thread runtime polls the exported futures, like Swift's executor does).

use crate::*;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Duration;

#[derive(Default)]
struct Collector {
    events: Mutex<Vec<FfiEvent>>,
}

impl EventListener for Collector {
    fn on_events(&self, events: Vec<FfiEvent>) {
        self.events.lock().extend(events);
    }
}

fn config(dir: &std::path::Path) -> FfiEngineConfig {
    FfiEngineConfig {
        data_dir: Some(dir.join("data").display().to_string()),
        headless: true,
        log_level: "warn".into(),
        app_version: "0.0.0-test".into(),
        bundle_id: "app.osprey.test".into(),
        download_dir: Some(dir.join("downloads").display().to_string()),
        start_local_api: false,
        skip_instance_lock: true,
    }
}

fn request(url: &str) -> FfiNewTaskRequest {
    FfiNewTaskRequest {
        url: Some(url.into()),
        mirrors: vec![],
        magnet: None,
        torrent_base64: None,
        metalink_url: None,
        hls_playlist_url: None,
        name: Some("sample.bin".into()),
        directory: None,
        queue_id: None,
        category_id: None,
        schedule_id: None,
        priority: None,
        tags: vec!["ffi-test".into()],
        options: None,
        start: false,
        origin: "test".into(),
        selected_files: None,
        referer_page: None,
    }
}

#[test]
fn open_list_rows_and_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let engine = OspreyEngine::open(config(dir.path())).expect("engine opens");
    // Plays the role of the Swift executor.
    let foreign = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let collector = Arc::new(Collector::default());
    let listener_id = engine.add_listener(collector.clone());
    assert!(listener_id > 0);

    let snap = foreign.block_on(engine.snapshot()).unwrap();
    assert_eq!(snap.rev, 1);
    assert!(snap.rows.is_empty());
    assert!(snap.queues.iter().any(|q| q.id == "queue-default"));
    assert!(!snap.categories.is_empty());

    let added = foreign
        .block_on(engine.add_task(request("http://127.0.0.1:9/sample.bin")))
        .expect("task added");
    assert_eq!(added.row.name, "sample.bin");

    let rows = foreign
        .block_on(engine.task_rows(FfiTaskFilter {
            text: None,
            states: vec![],
            kinds: vec![],
            queue_id: None,
            category_id: None,
            domain: None,
            tag: Some("ffi-test".into()),
            smart: None,
            created_since: None,
            min_size: None,
            max_size: None,
            sort: None,
            descending: false,
            limit: 0,
            offset: 0,
        }))
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, added.row.id);

    let detail = foreign
        .block_on(engine.task_detail(added.row.id.clone()))
        .unwrap();
    assert_eq!(
        detail.urls,
        vec!["http://127.0.0.1:9/sample.bin".to_string()]
    );

    // Typed errors cross as FfiError.
    match foreign.block_on(engine.task_detail("missing".into())) {
        Err(FfiError::NotFound { .. }) => {}
        other => panic!("expected NotFound, got {other:?}"),
    }

    // JSON surfaces round-trip.
    let settings = foreign
        .block_on(engine.update_settings_json(r#"{"bandwidth":{}}"#.into()))
        .unwrap();
    assert!(settings.contains("\"remote\""));
    let rule = foreign
        .block_on(engine.save_rule_json(
            r#"{"name":"ISOs","conditions":[{"field":"extension","any_of":["iso"]}]}"#.into(),
        ))
        .unwrap();
    assert!(rule.contains("ISOs"));

    // The forwarder delivers batched events on its own thread.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        let got = collector
            .events
            .lock()
            .iter()
            .any(|e| matches!(e, FfiEvent::TaskAdded { row } if row.id == added.row.id));
        if got {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        collector
            .events
            .lock()
            .iter()
            .any(|e| matches!(e, FfiEvent::TaskAdded { .. })),
        "TaskAdded delivered"
    );
    assert!(collector
        .events
        .lock()
        .iter()
        .any(|e| matches!(e, FfiEvent::RulesChanged)));

    let info = engine.info();
    assert!(info.data_dir.ends_with("data"));
    engine.remove_listener(listener_id);
    foreign.block_on(engine.shutdown()).unwrap();
    // Idempotent.
    foreign.block_on(engine.shutdown()).unwrap();
}

#[cfg(feature = "local-api")]
#[test]
fn local_api_socket_starts_and_stops() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(dir.path());
    cfg.start_local_api = true;
    let engine = OspreyEngine::open(cfg).expect("engine opens");
    let info = engine.info();
    let socket = info.socket_path.clone().expect("socket listener running");
    assert!(std::path::Path::new(&socket).exists(), "{socket}");
    let foreign = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    foreign.block_on(engine.shutdown()).unwrap();
    assert!(!std::path::Path::new(&socket).exists());
    assert!(engine.info().socket_path.is_none());
}
