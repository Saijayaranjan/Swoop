mod common;

use common::*;
use osprey_domain::history::{HistoryEntry, HistoryQuery, HistorySort};
use osprey_domain::*;
use osprey_store::Store;
use std::path::PathBuf;

fn entry(
    name: &str,
    url: &str,
    domain: &str,
    size: u64,
    finished: i64,
    tags: &[&str],
) -> HistoryEntry {
    let t = task_in_state(name, PathBuf::from("/dl"), TaskState::Completed);
    HistoryEntry {
        task_id: t.id,
        kind: TaskKind::Http,
        name: name.into(),
        original_url: url.into(),
        final_url: None,
        domain: domain.into(),
        size: Some(size),
        checksum: Some(Checksum::new(
            ChecksumAlgorithm::Sha256,
            format!("{:064x}", size),
        )),
        state: TaskState::Completed,
        destination: PathBuf::from("/dl").join(name),
        category_id: None,
        queue_id: QueueId::default_queue(),
        started_at: None,
        finished_at: Millis(finished),
        duration_seconds: size / 100,
        average_speed: size,
        peak_speed: size * 2,
        error: None,
        tags: tags.iter().map(|s| s.to_string()).collect(),
        origin: "cli".into(),
    }
}

async fn seeded() -> (std::sync::Arc<Store>, Vec<HistoryEntry>) {
    let store = Store::open_in_memory().unwrap();
    let rows = vec![
        entry(
            "ubuntu-24.04.iso",
            "https://releases.ubuntu.com/24.04/ubuntu-24.04.iso",
            "releases.ubuntu.com",
            5_000,
            100,
            &["linux", "iso"],
        ),
        entry(
            "report.pdf",
            "https://docs.example.com/q3/report.pdf",
            "docs.example.com",
            200,
            200,
            &["work"],
        ),
        entry(
            "song.mp3",
            "https://music.example.com/song.mp3",
            "music.example.com",
            3_000,
            300,
            &[],
        ),
        entry(
            "Ubuntu Guide.epub",
            "https://books.example.com/ubuntu-guide",
            "books.example.com",
            900,
            400,
            &["linux"],
        ),
    ];
    for r in &rows {
        store.insert_history(r).await.unwrap();
    }
    (store, rows)
}

#[tokio::test]
async fn round_trip_and_replace() {
    let (store, rows) = seeded().await;
    let all = store
        .query_history(&HistoryQuery {
            descending: false,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(all, rows);
    // replacing by task id keeps one row and updates the FTS index
    let mut changed = rows[0].clone();
    changed.name = "renamed-thing.iso".into();
    store.insert_history(&changed).await.unwrap();
    assert_eq!(store.count_history(&Default::default()).await.unwrap(), 4);
    let q = HistoryQuery {
        text: Some("renamed".into()),
        ..Default::default()
    };
    assert_eq!(store.query_history(&q).await.unwrap(), vec![changed]);
    let q = HistoryQuery {
        text: Some("ubuntu-24".into()),
        ..Default::default()
    };
    // still findable through the URL column
    assert_eq!(store.query_history(&q).await.unwrap().len(), 1);
}

#[tokio::test]
async fn fts_search() {
    let (store, rows) = seeded().await;
    let names = |v: Vec<HistoryEntry>| v.into_iter().map(|h| h.name).collect::<Vec<_>>();
    let search = |text: &str| HistoryQuery {
        text: Some(text.into()),
        descending: true,
        ..Default::default()
    };
    // name (case-insensitive, prefix), newest first
    assert_eq!(
        names(store.query_history(&search("ubu")).await.unwrap()),
        ["Ubuntu Guide.epub", "ubuntu-24.04.iso"]
    );
    // domain
    assert_eq!(
        names(
            store
                .query_history(&search("music.example.com"))
                .await
                .unwrap()
        ),
        ["song.mp3"]
    );
    // tag
    assert_eq!(
        names(store.query_history(&search("work")).await.unwrap()),
        ["report.pdf"]
    );
    // url path token
    assert_eq!(
        names(store.query_history(&search("q3")).await.unwrap()),
        ["report.pdf"]
    );
    // multiple tokens are AND-ed
    assert_eq!(
        names(store.query_history(&search("linux guide")).await.unwrap()),
        ["Ubuntu Guide.epub"]
    );
    // punctuation and quotes never break the query
    assert!(store
        .query_history(&search("\"(unbalanced OR NOT"))
        .await
        .unwrap()
        .is_empty());
    assert_eq!(store.count_history(&search("ubu")).await.unwrap(), 2);
    assert_eq!(rows.len(), 4);
}

#[tokio::test]
async fn filters_sort_and_paging() {
    let (store, _) = seeded().await;
    let names = |v: Vec<HistoryEntry>| v.into_iter().map(|h| h.name).collect::<Vec<_>>();
    let q = HistoryQuery {
        domain: Some("Docs.Example.com".into()),
        ..Default::default()
    };
    assert_eq!(
        names(store.query_history(&q).await.unwrap()),
        ["report.pdf"]
    );
    let q = HistoryQuery {
        tag: Some("linux".into()),
        sort: HistorySort::Name,
        ..Default::default()
    };
    // NOCASE: "ubuntu " < "ubuntu-" because space sorts before '-'
    assert_eq!(
        names(store.query_history(&q).await.unwrap()),
        ["Ubuntu Guide.epub", "ubuntu-24.04.iso"]
    );
    let q = HistoryQuery {
        since: Some(Millis(200)),
        until: Some(Millis(400)),
        sort: HistorySort::FinishedAt,
        descending: true,
        ..Default::default()
    };
    assert_eq!(
        names(store.query_history(&q).await.unwrap()),
        ["song.mp3", "report.pdf"]
    );
    let q = HistoryQuery {
        min_size: Some(900),
        max_size: Some(3_000),
        sort: HistorySort::Size,
        descending: true,
        ..Default::default()
    };
    assert_eq!(
        names(store.query_history(&q).await.unwrap()),
        ["song.mp3", "Ubuntu Guide.epub"]
    );
    let q = HistoryQuery {
        sort: HistorySort::Speed,
        descending: true,
        limit: 2,
        offset: 1,
        ..Default::default()
    };
    assert_eq!(
        names(store.query_history(&q).await.unwrap()),
        ["song.mp3", "Ubuntu Guide.epub"]
    );
    let q = HistoryQuery {
        state: Some(TaskState::Failed),
        ..Default::default()
    };
    assert_eq!(store.count_history(&q).await.unwrap(), 0);
    let q = HistoryQuery {
        kind: Some(TaskKind::Http),
        queue_id: Some(QueueId::default_queue()),
        sort: HistorySort::Duration,
        ..Default::default()
    };
    assert_eq!(store.count_history(&q).await.unwrap(), 4);
    assert_eq!(
        names(store.query_history(&q).await.unwrap())[0],
        "report.pdf"
    );
}

#[tokio::test]
async fn duplicate_lookups() {
    let (store, rows) = seeded().await;
    let by_sum = store
        .find_history_by_checksum(ChecksumAlgorithm::Sha256, &format!("{:064X}", 5_000))
        .await
        .unwrap();
    assert_eq!(by_sum, vec![rows[0].clone()]);
    assert!(store
        .find_history_by_checksum(ChecksumAlgorithm::Md5, &format!("{:064x}", 5_000))
        .await
        .unwrap()
        .is_empty());
    let by_url = store
        .find_history_by_url("https://docs.example.com/q3/report.pdf")
        .await
        .unwrap();
    assert_eq!(by_url, vec![rows[1].clone()]);
    let mut with_final = rows[2].clone();
    with_final.final_url = Some("https://cdn.example.com/s.mp3".into());
    store.insert_history(&with_final).await.unwrap();
    assert_eq!(
        store
            .find_history_by_url("https://cdn.example.com/s.mp3")
            .await
            .unwrap(),
        vec![with_final]
    );
    assert_eq!(
        store
            .find_history_by_name_size("song.mp3", 3_000)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(store
        .find_history_by_name_size("song.mp3", 3_001)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn delete_and_clear() {
    let (store, rows) = seeded().await;
    let n = store
        .delete_history(&[
            rows[0].task_id.clone(),
            rows[1].task_id.clone(),
            TaskId::new(),
        ])
        .await
        .unwrap();
    assert_eq!(n, 2);
    assert_eq!(store.count_history(&Default::default()).await.unwrap(), 2);
    // FTS index followed the delete
    let q = HistoryQuery {
        text: Some("ubuntu-24".into()),
        ..Default::default()
    };
    assert!(store.query_history(&q).await.unwrap().is_empty());
    assert_eq!(store.clear_history().await.unwrap(), 2);
    assert_eq!(store.count_history(&Default::default()).await.unwrap(), 0);
    let q = HistoryQuery {
        text: Some("song".into()),
        ..Default::default()
    };
    assert!(store.query_history(&q).await.unwrap().is_empty());
}
