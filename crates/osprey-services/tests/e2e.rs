use osprey_domain::{Event, NewTaskRequest, TaskState};
use osprey_services::bootstrap::{start, EngineConfig};
use osprey_testserver::{content_for, TestServer};
use std::time::Duration;

async fn wait_state(
    engine: &osprey_services::SharedEngine,
    id: &osprey_domain::TaskId,
    want: TaskState,
    secs: u64,
) -> osprey_domain::Task {
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    loop {
        let t = engine.get_task(id.clone()).await.unwrap();
        if t.state == want {
            return t;
        }
        if std::time::Instant::now() > deadline {
            panic!(
                "task stuck in {:?} (wanted {want:?}): error={:?}",
                t.state, t.error
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn http_download_end_to_end() {
    std::env::set_var("OSPREY_CREDENTIALS_FILE_STORE", "1");
    let server = TestServer::start().await;
    let data_dir = tempfile::tempdir().unwrap();
    let dl_dir = tempfile::tempdir().unwrap();
    let engine = start(EngineConfig {
        data_dir: Some(data_dir.path().into()),
        download_dir: Some(dl_dir.path().into()),
        headless: true,
        skip_instance_lock: true,
        ..Default::default()
    })
    .await
    .unwrap();
    let mut rx = engine.subscribe();
    let url = server.file_url("movie.bin", 3 * 1024 * 1024);
    let r = engine
        .add_task(NewTaskRequest {
            url: Some(url),
            start: true,
            origin: "test".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(r.duplicate.is_none());
    let t = wait_state(&engine, &r.task.id, TaskState::Completed, 30).await;
    let path = t.file_path.clone().expect("file path");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        content_for("movie.bin", 3 * 1024 * 1024)
    );
    let mut saw = (false, false, false);
    while let Ok(ev) = rx.try_recv() {
        match &*ev {
            Event::TaskAdded(_) => saw.0 = true,
            Event::Progress(_) => saw.1 = true,
            Event::TaskStateChanged {
                to: TaskState::Completed,
                ..
            } => saw.2 = true,
            _ => {}
        }
    }
    assert!(saw.0 && saw.2, "events {saw:?}");
    let h = engine.history(Default::default()).await.unwrap();
    assert_eq!(h.len(), 1);
    engine.shutdown().await;
}

/// Removing an unfinished task "with files" must never delete a same-named file that the user
/// already had: Osprey only created the part file, not `<directory>/<name>`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remove_with_files_keeps_unrelated_user_file() {
    std::env::set_var("OSPREY_CREDENTIALS_FILE_STORE", "1");
    let data_dir = tempfile::tempdir().unwrap();
    let dl_dir = tempfile::tempdir().unwrap();
    let mine = dl_dir.path().join("report.pdf");
    std::fs::write(&mine, b"the user's own file").unwrap();
    let engine = start(EngineConfig {
        data_dir: Some(data_dir.path().into()),
        download_dir: Some(dl_dir.path().into()),
        headless: true,
        skip_instance_lock: true,
        ..Default::default()
    })
    .await
    .unwrap();
    let r = engine
        .add_task(NewTaskRequest {
            url: Some("https://example.invalid/report.pdf".into()),
            name: Some("report.pdf".into()),
            start: false,
            origin: "test".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    engine.remove_task(r.task.id.clone(), true).await.unwrap();
    assert_eq!(std::fs::read(&mine).unwrap(), b"the user's own file");
    engine.shutdown().await;
}
