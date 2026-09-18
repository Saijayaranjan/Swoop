//! Shared fixtures for the integration tests.
#![allow(dead_code)]

use osprey_domain::health::HealthScore;
use osprey_domain::history::HistoryEntry;
use osprey_domain::state::PauseReason;
use osprey_domain::*;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A task with every optional field populated so round-trips exercise every column.
pub fn full_task(name: &str, dir: PathBuf) -> Task {
    let mut t = Task::new(
        TaskKind::Http,
        Source::Urls {
            urls: vec![
                format!("https://Example.com/files/{name}?x=1"),
                format!("https://mirror.example.org/{name}"),
            ],
        },
        name,
        dir,
        QueueId::default_queue(),
    );
    t.file_path = Some(t.directory.join(name));
    t.blocked_by = vec![PauseReason::User, PauseReason::Queue("q".into())];
    t.rev = 7;
    t.name_locked = true;
    t.status_detail = Some("probing".into());
    t.error = Some(
        TaskError::from_http_status(503, "https://example.com/x")
            .with_detail("d")
            .with_retry_after_ms(1500),
    );
    t.progress = Progress {
        downloaded: 1234,
        uploaded: 5,
        total: Some(10_000),
        fraction: 0.1234,
        ratio: 0.5,
        ..Default::default()
    };
    t.stats = TaskStats {
        average_speed: 100,
        peak_speed: 200,
        retries: 2,
        range_supported: Some(true),
        etag: Some("\"abc\"".into()),
        final_url: Some("https://cdn.example.com/x".into()),
        ..Default::default()
    };
    t.category_id = Some(CategoryId("cat-archives".into()));
    t.schedule_id = Some(ScheduleId("sched-1".into()));
    t.priority = Priority::High;
    t.position = 42;
    t.tags = vec!["linux".into(), "iso".into()];
    t.options = TaskOptions {
        max_connections: Some(4),
        headers: BTreeMap::from([("X-Test".to_string(), "1".to_string())]),
        checksum: Some(Checksum::new(ChecksumAlgorithm::Sha256, "ab".repeat(32))),
        conflict_policy: ConflictPolicy::Rename,
        seed_ratio_limit: Some(1.5),
        open_when_done: true,
        ..Default::default()
    };
    t.segment_map = Some(SegmentMap {
        segments: vec![Segment::new(0, 0, 5000), Segment::new(1, 5000, 10_000)],
        etag: Some("\"abc\"".into()),
        last_modified: Some("Mon".into()),
        total: Some(10_000),
        part_path: Some(t.directory.join(format!("{name}.osprey-part"))),
    });
    t.media = Some(osprey_domain::media::MediaInfo {
        kind: osprey_domain::media::MediaKind::Video,
        title: Some("t".into()),
        format: None,
        duration_seconds: Some(1.5),
        variants: vec![],
        selected_variant: None,
        segment_count: Some(3),
        segments_done: 0,
        protected: false,
        page_url: None,
    });
    t.torrent = Some(osprey_domain::torrent::TorrentInfo {
        info_hash: "ff".repeat(20),
        name: "tor".into(),
        ..Default::default()
    });
    t.health = HealthScore {
        score: 77,
        notes: vec!["health.good".into()],
        ..Default::default()
    };
    t.origin = "browser".into();
    t.mime = Some("application/octet-stream".into());
    t.started_at = Some(Millis(10));
    t.completed_at = None;
    t.verified_checksum = Some(Checksum::new(ChecksumAlgorithm::Sha1, "cd".repeat(20)));
    t.attempt = 3;
    t.next_retry_at = Some(Millis(99));
    t
}

/// A minimal task in `state`, reached through legal transitions.
pub fn task_in_state(name: &str, dir: PathBuf, state: TaskState) -> Task {
    let mut t = Task::new(
        TaskKind::Http,
        Source::Urls {
            urls: vec![format!("https://example.com/{name}")],
        },
        name,
        dir,
        QueueId::default_queue(),
    );
    use TaskState::*;
    let path: &[TaskState] = match state {
        Pending => &[],
        Queued => &[Queued],
        Scheduled => &[Scheduled],
        Resolving => &[Queued, Resolving],
        Connecting => &[Queued, Connecting],
        Downloading => &[Queued, Connecting, Downloading],
        Paused => &[Queued, Paused],
        Retrying => &[Queued, Connecting, Retrying],
        Verifying => &[Queued, Connecting, Downloading, Verifying],
        Processing => &[Queued, Connecting, Downloading, Processing],
        Completed => &[Queued, Connecting, Downloading, Completed],
        Failed => &[Failed],
        Cancelled => &[Cancelled],
        Seeding => &[Queued, Connecting, Downloading, Seeding],
    };
    for s in path {
        t.transition(*s).unwrap();
    }
    t
}

pub fn history_for(task: &Task, name: &str) -> HistoryEntry {
    HistoryEntry {
        task_id: task.id.clone(),
        kind: task.kind,
        name: name.to_owned(),
        original_url: task.source.primary_url().unwrap_or("").to_owned(),
        final_url: Some("https://cdn.example.com/final".into()),
        domain: task.domain().unwrap_or_default(),
        size: Some(10_000),
        checksum: Some(Checksum::new(ChecksumAlgorithm::Sha256, "ab".repeat(32))),
        state: TaskState::Completed,
        destination: task.directory.join(name),
        category_id: task.category_id.clone(),
        queue_id: task.queue_id.clone(),
        started_at: Some(Millis(1)),
        finished_at: Millis(2_000),
        duration_seconds: 2,
        average_speed: 5_000,
        peak_speed: 9_000,
        error: None,
        tags: vec!["linux".into()],
        origin: "browser".into(),
    }
}
