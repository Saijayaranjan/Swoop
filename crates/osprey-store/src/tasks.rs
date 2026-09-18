//! Task persistence: the cold `tasks` row, the hot `task_progress` / `task_checkpoints` rows,
//! the per-task log and the raw `.torrent` blobs.
//!
//! `Task.progress` lives in `task_progress`; `Task.segment_map` is the `Segments` checkpoint in
//! `task_checkpoints`. Both are merged back into the `Task` on every read, so callers see one
//! record while the writer only churns the small hot rows at high frequency.

use crate::error::{StoreError, StoreResult};
use crate::reader::{
    enum_from_str, enum_to_str, from_json, from_json_opt, get_u64_opt, like_contains,
    path_opt_to_text, path_to_text, to_json, to_json_opt, u64_opt_to_sql, u64_to_sql, Params,
};
use crate::Store;
use osprey_domain::events::{LogLevel, TaskLogEntry};
use osprey_domain::history::HistoryEntry;
use osprey_domain::media::MediaInfo;
use osprey_domain::{
    CategoryId, Checkpoint, Millis, Priority, Progress, QueueId, Task, TaskId, TaskKind, TaskState,
};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Rows kept per task in `task_log`.
pub const TASK_LOG_KEEP: i64 = 500;

// ---------------------------------------------------------------------------------------------
// Filter
// ---------------------------------------------------------------------------------------------

/// Sort key for [`TaskFilter`]. Mirrors the services layer's `TaskSort`; `Speed` and `Eta` are
/// transient values that are not persisted, so they fall back to manual position order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskSort {
    /// Manual queue order.
    #[default]
    Position,
    /// Creation time.
    CreatedAt,
    /// Case-insensitive name.
    Name,
    /// Known total size.
    Size,
    /// Completed fraction.
    Progress,
    /// Not persisted; sorts by position.
    Speed,
    /// Not persisted; sorts by position.
    Eta,
    /// State name.
    State,
    /// Source domain.
    Domain,
}

/// Filter for [`Store::list_tasks`] / [`Store::count_tasks`]. Field-compatible with the
/// services layer's `TaskFilter` so it can be converted without loss.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskFilter {
    /// Free text over name, URL, domain and tags (substring, case-insensitive).
    pub text: Option<String>,
    /// Any of these states (empty = all).
    pub states: Vec<TaskState>,
    /// Any of these kinds (empty = all).
    pub kinds: Vec<TaskKind>,
    /// Restrict to one queue.
    pub queue_id: Option<QueueId>,
    /// Restrict to one category.
    pub category_id: Option<CategoryId>,
    /// Exact source domain.
    pub domain: Option<String>,
    /// Exact tag.
    pub tag: Option<String>,
    /// UI smart filter: `active`, `queued`, `scheduled`, `complete`, `failed`, `torrent`,
    /// `media`, `paused`. Unknown values are ignored.
    pub smart: Option<String>,
    /// Only tasks created at or after this instant.
    pub created_since: Option<Millis>,
    /// Minimum known total size.
    pub min_size: Option<u64>,
    /// Maximum known total size.
    pub max_size: Option<u64>,
    /// Sort key.
    pub sort: TaskSort,
    /// Reverse the sort.
    pub descending: bool,
    /// Page size; `0` = unlimited.
    pub limit: u32,
    /// Page offset.
    pub offset: u32,
}

impl TaskFilter {
    /// Build the `WHERE` clause (without the keyword) and its parameters.
    fn where_clause(&self) -> (String, Params) {
        let mut conds: Vec<String> = Vec::new();
        let mut p = Params::default();
        let mut states: BTreeSet<&'static str> = self.states.iter().map(|s| s.as_str()).collect();
        let mut kinds: BTreeSet<&'static str> = self.kinds.iter().map(|k| k.as_str()).collect();
        match self.smart.as_deref() {
            Some("active") => states.extend(
                TaskState::ALL
                    .iter()
                    .filter(|s| s.is_active())
                    .map(|s| s.as_str()),
            ),
            Some("queued") => {
                states.insert(TaskState::Queued.as_str());
            }
            Some("scheduled") => {
                states.insert(TaskState::Scheduled.as_str());
            }
            Some("complete") => {
                states.insert(TaskState::Completed.as_str());
            }
            Some("failed") => {
                states.insert(TaskState::Failed.as_str());
            }
            Some("paused") => {
                states.insert(TaskState::Paused.as_str());
            }
            Some("torrent") => {
                kinds.insert(TaskKind::Torrent.as_str());
                kinds.insert(TaskKind::Magnet.as_str());
            }
            Some("media") => conds.push("(t.kind = 'hls' OR t.media_json IS NOT NULL)".into()),
            _ => {}
        }
        if !states.is_empty() {
            let marks = vec!["?"; states.len()].join(",");
            conds.push(format!("t.state IN ({marks})"));
            for s in states {
                p.text(s);
            }
        }
        if !kinds.is_empty() {
            let marks = vec!["?"; kinds.len()].join(",");
            conds.push(format!("t.kind IN ({marks})"));
            for k in kinds {
                p.text(k);
            }
        }
        if let Some(q) = &self.queue_id {
            conds.push("t.queue_id = ?".into());
            p.text(q.as_str());
        }
        if let Some(c) = &self.category_id {
            conds.push("t.category_id = ?".into());
            p.text(c.as_str());
        }
        if let Some(d) = &self.domain {
            conds.push("t.domain = ?".into());
            p.text(d.to_lowercase());
        }
        if let Some(tag) = &self.tag {
            conds.push("EXISTS (SELECT 1 FROM json_each(t.tags_json) WHERE value = ?)".into());
            p.text(tag.as_str());
        }
        if let Some(text) = self
            .text
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            let like = like_contains(text);
            conds.push(
                "(t.name LIKE ? ESCAPE '\\' OR t.url LIKE ? ESCAPE '\\' \
                 OR t.domain LIKE ? ESCAPE '\\' OR t.tags_json LIKE ? ESCAPE '\\')"
                    .into(),
            );
            for _ in 0..4 {
                p.text(like.clone());
            }
        }
        if let Some(since) = self.created_since {
            conds.push("t.created_at >= ?".into());
            p.int(since.0);
        }
        if let Some(min) = self.min_size {
            conds.push("COALESCE(p.total, 0) >= ?".into());
            p.int(u64_to_sql(min));
        }
        if let Some(max) = self.max_size {
            conds.push("COALESCE(p.total, 0) <= ?".into());
            p.int(u64_to_sql(max));
        }
        let sql = if conds.is_empty() {
            "1 = 1".to_owned()
        } else {
            conds.join(" AND ")
        };
        (sql, p)
    }

    fn order_clause(&self) -> String {
        let dir = if self.descending { "DESC" } else { "ASC" };
        let key = match self.sort {
            TaskSort::Position | TaskSort::Speed | TaskSort::Eta => "t.position",
            TaskSort::CreatedAt => "t.created_at",
            TaskSort::Name => "t.name COLLATE NOCASE",
            TaskSort::Size => "COALESCE(p.total, 0)",
            TaskSort::Progress => {
                "CASE WHEN COALESCE(p.total, 0) > 0 \
                 THEN CAST(p.downloaded AS REAL) / p.total ELSE COALESCE(p.fraction, 0) END"
            }
            TaskSort::State => "t.state",
            TaskSort::Domain => "t.domain",
        };
        format!("ORDER BY {key} {dir}, t.created_at {dir}, t.id ASC")
    }
}

// ---------------------------------------------------------------------------------------------
// Row mapping
// ---------------------------------------------------------------------------------------------

/// Columns selected for every task read; the joins bring in the hot rows.
const TASK_SELECT: &str = "SELECT t.id, t.rev, t.kind, t.state, t.queue_id, t.category_id, \
    t.schedule_id, t.priority, t.position, t.name, t.directory, t.file_path, t.origin, t.mime, \
    t.created_at, t.updated_at, t.started_at, t.completed_at, t.attempt, t.next_retry_at, \
    t.name_locked, t.directory_locked, t.status_detail, t.source_json, t.options_json, \
    t.error_json, t.stats_json, t.health_json, t.media_json, t.torrent_json, t.tags_json, \
    t.blocked_json, t.verified_checksum_json, \
    p.downloaded, p.uploaded, p.total, p.fraction, p.ratio, \
    c.blob AS checkpoint_blob \
    FROM tasks t \
    LEFT JOIN task_progress p ON p.task_id = t.id \
    LEFT JOIN task_checkpoints c ON c.task_id = t.id";

/// Map one joined row to a `Task` (with progress and segment map merged) plus the raw
/// checkpoint, if any.
fn task_from_row(row: &Row<'_>) -> StoreResult<(Task, Option<Checkpoint>)> {
    let kind_s: String = row.get("kind")?;
    let kind = TaskKind::parse(&kind_s)
        .ok_or_else(|| StoreError::Corrupt(format!("kind: unknown value {kind_s}")))?;
    let state_s: String = row.get("state")?;
    let state = TaskState::parse(&state_s)
        .ok_or_else(|| StoreError::Corrupt(format!("state: unknown value {state_s}")))?;
    let source_json: String = row.get("source_json")?;
    let options_json: String = row.get("options_json")?;
    let stats_json: String = row.get("stats_json")?;
    let health_json: String = row.get("health_json")?;
    let tags_json: String = row.get("tags_json")?;
    let blocked_json: String = row.get("blocked_json")?;
    let mut media: Option<MediaInfo> = from_json_opt("media_json", row.get("media_json")?)?;

    let checkpoint: Option<Checkpoint> = row
        .get::<_, Option<Vec<u8>>>("checkpoint_blob")?
        .map(|b| {
            serde_json::from_slice(&b)
                .map_err(|e| StoreError::Corrupt(format!("task_checkpoints.blob: {e}")))
        })
        .transpose()?;
    let segment_map = match &checkpoint {
        Some(Checkpoint::Segments(m)) => Some(m.clone()),
        _ => None,
    };
    if let (Some(Checkpoint::Hls(h)), Some(m)) = (&checkpoint, media.as_mut()) {
        m.segments_done = h.done_count();
    }

    let progress = Progress {
        downloaded: get_u64_opt(row, "downloaded")?.unwrap_or(0),
        uploaded: get_u64_opt(row, "uploaded")?.unwrap_or(0),
        total: get_u64_opt(row, "total")?,
        fraction: row.get::<_, Option<f64>>("fraction")?.unwrap_or(0.0) as f32,
        ratio: row.get::<_, Option<f64>>("ratio")?.unwrap_or(0.0) as f32,
        ..Progress::default()
    };

    let task = Task {
        id: TaskId(row.get("id")?),
        kind,
        source: from_json("source_json", &source_json)?,
        name: row.get("name")?,
        directory: row.get::<_, String>("directory")?.into(),
        file_path: row.get::<_, Option<String>>("file_path")?.map(Into::into),
        state,
        blocked_by: from_json("blocked_json", &blocked_json)?,
        rev: row.get::<_, i64>("rev")? as u64,
        name_locked: row.get("name_locked")?,
        directory_locked: row.get("directory_locked")?,
        status_detail: row.get("status_detail")?,
        error: from_json_opt("error_json", row.get("error_json")?)?,
        progress,
        stats: from_json("stats_json", &stats_json)?,
        queue_id: QueueId(row.get("queue_id")?),
        category_id: row.get::<_, Option<String>>("category_id")?.map(CategoryId),
        schedule_id: row
            .get::<_, Option<String>>("schedule_id")?
            .map(osprey_domain::ScheduleId),
        priority: Priority::from_i32(row.get("priority")?),
        position: row.get("position")?,
        tags: from_json("tags_json", &tags_json)?,
        options: from_json("options_json", &options_json)?,
        segment_map,
        torrent: from_json_opt("torrent_json", row.get("torrent_json")?)?,
        media,
        health: from_json("health_json", &health_json)?,
        origin: row.get("origin")?,
        mime: row.get("mime")?,
        created_at: Millis(row.get("created_at")?),
        updated_at: Millis(row.get("updated_at")?),
        started_at: row.get::<_, Option<i64>>("started_at")?.map(Millis),
        completed_at: row.get::<_, Option<i64>>("completed_at")?.map(Millis),
        verified_checksum: from_json_opt(
            "verified_checksum_json",
            row.get("verified_checksum_json")?,
        )?,
        attempt: row.get::<_, i64>("attempt")?.max(0) as u32,
        next_retry_at: row.get::<_, Option<i64>>("next_retry_at")?.map(Millis),
    };
    Ok((task, checkpoint))
}

/// Owned column values for the cold row, bound by name in INSERT/UPDATE statements.
struct ColdRow {
    id: String,
    rev: i64,
    kind: &'static str,
    state: &'static str,
    queue_id: String,
    category_id: Option<String>,
    schedule_id: Option<String>,
    priority: i32,
    position: i64,
    name: String,
    directory: String,
    file_path: Option<String>,
    origin: String,
    mime: Option<String>,
    url: Option<String>,
    domain: Option<String>,
    created_at: i64,
    updated_at: i64,
    started_at: Option<i64>,
    completed_at: Option<i64>,
    attempt: i64,
    next_retry_at: Option<i64>,
    name_locked: bool,
    directory_locked: bool,
    status_detail: Option<String>,
    source_json: String,
    options_json: String,
    error_json: Option<String>,
    stats_json: String,
    health_json: String,
    media_json: Option<String>,
    torrent_json: Option<String>,
    tags_json: String,
    blocked_json: String,
    verified_checksum_json: Option<String>,
}

impl ColdRow {
    fn from_task(t: &Task) -> StoreResult<Self> {
        Ok(Self {
            id: t.id.0.clone(),
            rev: t.rev as i64,
            kind: t.kind.as_str(),
            state: t.state.as_str(),
            queue_id: t.queue_id.0.clone(),
            category_id: t.category_id.as_ref().map(|c| c.0.clone()),
            schedule_id: t.schedule_id.as_ref().map(|s| s.0.clone()),
            priority: t.priority.as_i32(),
            position: t.position,
            name: t.name.clone(),
            directory: path_to_text(&t.directory),
            file_path: path_opt_to_text(&t.file_path),
            origin: t.origin.clone(),
            mime: t.mime.clone(),
            url: t.source.primary_url().map(str::to_owned),
            domain: t.source.domain(),
            created_at: t.created_at.0,
            updated_at: t.updated_at.0,
            started_at: t.started_at.map(|m| m.0),
            completed_at: t.completed_at.map(|m| m.0),
            attempt: i64::from(t.attempt),
            next_retry_at: t.next_retry_at.map(|m| m.0),
            name_locked: t.name_locked,
            directory_locked: t.directory_locked,
            status_detail: t.status_detail.clone(),
            source_json: to_json(&t.source)?,
            options_json: to_json(&t.options)?,
            error_json: to_json_opt(&t.error)?,
            stats_json: to_json(&t.stats)?,
            health_json: to_json(&t.health)?,
            media_json: to_json_opt(&t.media)?,
            torrent_json: to_json_opt(&t.torrent)?,
            tags_json: to_json(&t.tags)?,
            blocked_json: to_json(&t.blocked_by)?,
            verified_checksum_json: to_json_opt(&t.verified_checksum)?,
        })
    }

    fn named(&self) -> [(&'static str, &dyn rusqlite::ToSql); 35] {
        [
            (":id", &self.id),
            (":rev", &self.rev),
            (":kind", &self.kind),
            (":state", &self.state),
            (":queue_id", &self.queue_id),
            (":category_id", &self.category_id),
            (":schedule_id", &self.schedule_id),
            (":priority", &self.priority),
            (":position", &self.position),
            (":name", &self.name),
            (":directory", &self.directory),
            (":file_path", &self.file_path),
            (":origin", &self.origin),
            (":mime", &self.mime),
            (":url", &self.url),
            (":domain", &self.domain),
            (":created_at", &self.created_at),
            (":updated_at", &self.updated_at),
            (":started_at", &self.started_at),
            (":completed_at", &self.completed_at),
            (":attempt", &self.attempt),
            (":next_retry_at", &self.next_retry_at),
            (":name_locked", &self.name_locked),
            (":directory_locked", &self.directory_locked),
            (":status_detail", &self.status_detail),
            (":source_json", &self.source_json),
            (":options_json", &self.options_json),
            (":error_json", &self.error_json),
            (":stats_json", &self.stats_json),
            (":health_json", &self.health_json),
            (":media_json", &self.media_json),
            (":torrent_json", &self.torrent_json),
            (":tags_json", &self.tags_json),
            (":blocked_json", &self.blocked_json),
            (":verified_checksum_json", &self.verified_checksum_json),
        ]
    }
}

const COLD_COLUMNS: &str = "id, rev, kind, state, queue_id, category_id, schedule_id, priority, \
    position, name, directory, file_path, origin, mime, url, domain, created_at, updated_at, \
    started_at, completed_at, attempt, next_retry_at, name_locked, directory_locked, \
    status_detail, source_json, options_json, error_json, stats_json, health_json, media_json, \
    torrent_json, tags_json, blocked_json, verified_checksum_json";

const COLD_VALUES: &str = ":id, :rev, :kind, :state, :queue_id, :category_id, :schedule_id, \
    :priority, :position, :name, :directory, :file_path, :origin, :mime, :url, :domain, \
    :created_at, :updated_at, :started_at, :completed_at, :attempt, :next_retry_at, \
    :name_locked, :directory_locked, :status_detail, :source_json, :options_json, :error_json, \
    :stats_json, :health_json, :media_json, :torrent_json, :tags_json, :blocked_json, \
    :verified_checksum_json";

const COLD_SET: &str = "rev = :rev, kind = :kind, state = :state, queue_id = :queue_id, \
    category_id = :category_id, schedule_id = :schedule_id, priority = :priority, \
    position = :position, name = :name, directory = :directory, file_path = :file_path, \
    origin = :origin, mime = :mime, url = :url, domain = :domain, created_at = :created_at, \
    updated_at = :updated_at, started_at = :started_at, completed_at = :completed_at, \
    attempt = :attempt, next_retry_at = :next_retry_at, name_locked = :name_locked, \
    directory_locked = :directory_locked, status_detail = :status_detail, \
    source_json = :source_json, options_json = :options_json, error_json = :error_json, \
    stats_json = :stats_json, health_json = :health_json, media_json = :media_json, \
    torrent_json = :torrent_json, tags_json = :tags_json, blocked_json = :blocked_json, \
    verified_checksum_json = :verified_checksum_json";

// ---------------------------------------------------------------------------------------------
// Writer-thread primitives (also used by `complete_task` and recovery)
// ---------------------------------------------------------------------------------------------

/// Insert or replace the cold row.
pub(crate) fn upsert_cold_row(conn: &Connection, task: &Task) -> StoreResult<()> {
    let row = ColdRow::from_task(task)?;
    let set: Vec<String> = COLD_COLUMNS
        .split(',')
        .map(str::trim)
        .filter(|c| *c != "id")
        .map(|c| format!("{c} = excluded.{c}"))
        .collect();
    let sql = format!(
        "INSERT INTO tasks ({COLD_COLUMNS}) VALUES ({COLD_VALUES}) \
         ON CONFLICT(id) DO UPDATE SET {}",
        set.join(", ")
    );
    conn.prepare_cached(&sql)?.execute(&row.named()[..])?;
    Ok(())
}

/// Update the cold row; `NotFound` if the task does not exist.
pub(crate) fn update_cold_row(conn: &Connection, task: &Task) -> StoreResult<()> {
    let row = ColdRow::from_task(task)?;
    let sql = format!("UPDATE tasks SET {COLD_SET} WHERE id = :id");
    let n = conn.prepare_cached(&sql)?.execute(&row.named()[..])?;
    if n == 0 {
        return Err(StoreError::NotFound(format!("task {}", task.id)));
    }
    Ok(())
}

/// Write the cheap state subset of the cold row.
pub(crate) fn update_state_row(conn: &Connection, task: &Task) -> StoreResult<()> {
    let n = conn
        .prepare_cached(
            "UPDATE tasks SET state = ?2, rev = ?3, updated_at = ?4, blocked_json = ?5, \
             error_json = ?6, next_retry_at = ?7, attempt = ?8, started_at = ?9, \
             completed_at = ?10 WHERE id = ?1",
        )?
        .execute(params![
            task.id.as_str(),
            task.state.as_str(),
            task.rev as i64,
            task.updated_at.0,
            to_json(&task.blocked_by)?,
            to_json_opt(&task.error)?,
            task.next_retry_at.map(|m| m.0),
            i64::from(task.attempt),
            task.started_at.map(|m| m.0),
            task.completed_at.map(|m| m.0),
        ])?;
    if n == 0 {
        return Err(StoreError::NotFound(format!("task {}", task.id)));
    }
    Ok(())
}

/// Upsert the hot progress row (no-op if the task row is gone).
pub(crate) fn upsert_progress_row(
    conn: &Connection,
    task_id: &TaskId,
    p: &Progress,
    now: Millis,
) -> StoreResult<()> {
    conn.prepare_cached(
        "INSERT INTO task_progress (task_id, downloaded, uploaded, total, fraction, ratio, \
         updated_at) \
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7 WHERE EXISTS (SELECT 1 FROM tasks WHERE id = ?1) \
         ON CONFLICT(task_id) DO UPDATE SET downloaded = excluded.downloaded, \
         uploaded = excluded.uploaded, total = excluded.total, fraction = excluded.fraction, \
         ratio = excluded.ratio, updated_at = excluded.updated_at",
    )?
    .execute(params![
        task_id.as_str(),
        u64_to_sql(p.downloaded),
        u64_to_sql(p.uploaded),
        u64_opt_to_sql(p.total),
        f64::from(p.fraction),
        f64::from(p.ratio),
        now.0,
    ])?;
    Ok(())
}

/// Upsert the checkpoint row.
pub(crate) fn upsert_checkpoint_row(
    conn: &Connection,
    task_id: &TaskId,
    cp: &Checkpoint,
    now: Millis,
) -> StoreResult<()> {
    let blob = serde_json::to_vec(cp)?;
    let (etag, last_modified, total, part_path) = match cp {
        Checkpoint::Segments(m) => (
            m.etag.clone(),
            m.last_modified.clone(),
            u64_opt_to_sql(m.total),
            path_opt_to_text(&m.part_path),
        ),
        Checkpoint::Hls(h) => (None, None, None, Some(path_to_text(&h.part_dir))),
        Checkpoint::Torrent(t) => (None, None, None, Some(path_to_text(&t.output_folder))),
    };
    conn.prepare_cached(
        "INSERT INTO task_checkpoints (task_id, kind, blob, etag, last_modified, total, \
         part_path, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
         ON CONFLICT(task_id) DO UPDATE SET kind = excluded.kind, blob = excluded.blob, \
         etag = excluded.etag, last_modified = excluded.last_modified, total = excluded.total, \
         part_path = excluded.part_path, updated_at = excluded.updated_at",
    )?
    .execute(params![
        task_id.as_str(),
        cp.kind_name(),
        blob,
        etag,
        last_modified,
        total,
        part_path,
        now.0,
    ])?;
    Ok(())
}

/// Delete the checkpoint row; returns whether one existed.
pub(crate) fn delete_checkpoint_row(conn: &Connection, task_id: &TaskId) -> StoreResult<bool> {
    let n = conn
        .prepare_cached("DELETE FROM task_checkpoints WHERE task_id = ?1")?
        .execute([task_id.as_str()])?;
    Ok(n > 0)
}

/// Load every task (with checkpoints) on the given connection.
pub(crate) fn load_all_on(conn: &Connection) -> StoreResult<Vec<(Task, Option<Checkpoint>)>> {
    let sql = format!("{TASK_SELECT} ORDER BY t.position ASC, t.created_at ASC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |r| Ok(task_from_row(r)))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r??);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------------------------

impl Store {
    /// Persist a new task: cold row, its progress, and — when `segment_map` is set — a
    /// `Segments` checkpoint. Replaces an existing row with the same id (import/overwrite).
    pub async fn insert_task(&self, task: &Task) -> StoreResult<()> {
        let task = task.clone();
        self.write("insert_task", move |conn| {
            upsert_cold_row(conn, &task)?;
            let now = Millis::now();
            upsert_progress_row(conn, &task.id, &task.progress, now)?;
            if let Some(map) = &task.segment_map {
                upsert_checkpoint_row(conn, &task.id, &Checkpoint::Segments(map.clone()), now)?;
            }
            Ok(())
        })
        .await
    }

    /// Rewrite the full cold row. Progress and checkpoints are untouched.
    pub async fn update_task(&self, task: &Task) -> StoreResult<()> {
        let task = task.clone();
        self.write("update_task", move |conn| update_cold_row(conn, &task))
            .await
    }

    /// Cheap state update: `state`, `rev`, `updated_at`, `blocked_by`, `error`,
    /// `next_retry_at`, `attempt` (plus `started_at`/`completed_at`, which transitions set).
    pub async fn update_task_state(&self, task: &Task) -> StoreResult<()> {
        let task = task.clone();
        self.write("update_task_state", move |conn| {
            update_state_row(conn, &task)
        })
        .await
    }

    /// Delete a task and (by cascade) its progress, checkpoint and log. Returns whether a row
    /// was removed.
    pub async fn delete_task(&self, id: &TaskId) -> StoreResult<bool> {
        let id = id.clone();
        self.write("delete_task", move |conn| {
            let n = conn
                .prepare_cached("DELETE FROM tasks WHERE id = ?1")?
                .execute([id.as_str()])?;
            Ok(n > 0)
        })
        .await
    }

    /// Fetch one task with progress and segment map merged.
    pub async fn get_task(&self, id: &TaskId) -> StoreResult<Option<Task>> {
        let id = id.clone();
        self.read(move |conn| {
            let sql = format!("{TASK_SELECT} WHERE t.id = ?1");
            let mut stmt = conn.prepare_cached(&sql)?;
            let row = stmt
                .query_row([id.as_str()], |r| Ok(task_from_row(r)))
                .optional()?;
            Ok(row.transpose()?.map(|(t, _)| t))
        })
        .await
    }

    /// Every task, ordered by position — the startup load. Progress and checkpoints merged.
    pub async fn load_all_tasks(&self) -> StoreResult<Vec<Task>> {
        self.read(|conn| Ok(load_all_on(conn)?.into_iter().map(|(t, _)| t).collect()))
            .await
    }

    /// Like [`Store::load_all_tasks`] but also returns each task's raw checkpoint so engines
    /// can resume HLS/torrent state without a second query.
    pub async fn load_all_tasks_with_checkpoints(
        &self,
    ) -> StoreResult<Vec<(Task, Option<Checkpoint>)>> {
        self.read(load_all_on).await
    }

    /// Filtered, sorted, paged list.
    pub async fn list_tasks(&self, filter: &TaskFilter) -> StoreResult<Vec<Task>> {
        let filter = filter.clone();
        self.read(move |conn| {
            let (where_sql, mut params) = filter.where_clause();
            let order = filter.order_clause();
            let limit = if filter.limit == 0 {
                -1
            } else {
                i64::from(filter.limit)
            };
            params.int(limit).int(i64::from(filter.offset));
            let sql = format!("{TASK_SELECT} WHERE {where_sql} {order} LIMIT ? OFFSET ?");
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params_from_iter(params.0), |r| Ok(task_from_row(r)))?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r??.0);
            }
            Ok(out)
        })
        .await
    }

    /// Number of tasks matching the filter (ignores paging).
    pub async fn count_tasks(&self, filter: &TaskFilter) -> StoreResult<u64> {
        let filter = filter.clone();
        self.read(move |conn| {
            let (where_sql, params) = filter.where_clause();
            let sql = format!(
                "SELECT COUNT(*) FROM tasks t LEFT JOIN task_progress p ON p.task_id = t.id \
                 WHERE {where_sql}"
            );
            let n: i64 = conn.query_row(&sql, params_from_iter(params.0), |r| r.get(0))?;
            Ok(n.max(0) as u64)
        })
        .await
    }

    /// Deferred: write the hot progress row (latest wins). Callers debounce; see also
    /// [`Store::progress_writer`].
    pub async fn upsert_progress(&self, task_id: &TaskId, progress: &Progress) -> StoreResult<()> {
        let (id, p) = (task_id.clone(), progress.clone());
        self.write_deferred("upsert_progress", move |conn| {
            upsert_progress_row(conn, &id, &p, Millis::now())
        })
    }

    /// Persist a resume checkpoint (synchronous; the caller decides when).
    pub async fn upsert_checkpoint(
        &self,
        task_id: &TaskId,
        checkpoint: &Checkpoint,
    ) -> StoreResult<()> {
        let (id, cp) = (task_id.clone(), checkpoint.clone());
        self.write("upsert_checkpoint", move |conn| {
            upsert_checkpoint_row(conn, &id, &cp, Millis::now())
        })
        .await
    }

    /// Remove a task's checkpoint; returns whether one existed.
    pub async fn delete_checkpoint(&self, task_id: &TaskId) -> StoreResult<bool> {
        let id = task_id.clone();
        self.write("delete_checkpoint", move |conn| {
            delete_checkpoint_row(conn, &id)
        })
        .await
    }

    /// Fetch a task's checkpoint.
    pub async fn get_checkpoint(&self, task_id: &TaskId) -> StoreResult<Option<Checkpoint>> {
        let id = task_id.clone();
        self.read(move |conn| {
            let blob: Option<Vec<u8>> = conn
                .prepare_cached("SELECT blob FROM task_checkpoints WHERE task_id = ?1")?
                .query_row([id.as_str()], |r| r.get(0))
                .optional()?;
            blob.map(|b| {
                serde_json::from_slice(&b)
                    .map_err(|e| StoreError::Corrupt(format!("task_checkpoints.blob: {e}")))
            })
            .transpose()
        })
        .await
    }

    /// Deferred: append log lines (batched by the caller), trimming each task to
    /// [`TASK_LOG_KEEP`] rows. Lines for unknown tasks are dropped.
    pub async fn append_task_log(&self, entries: Vec<TaskLogEntry>) -> StoreResult<()> {
        if entries.is_empty() {
            return Ok(());
        }
        self.write_deferred("append_task_log", move |conn| {
            let mut touched: BTreeSet<String> = BTreeSet::new();
            {
                let mut ins = conn.prepare_cached(
                    "INSERT INTO task_log (task_id, at, level, code, message) \
                     SELECT ?1, ?2, ?3, ?4, ?5 WHERE EXISTS (SELECT 1 FROM tasks WHERE id = ?1)",
                )?;
                for e in &entries {
                    ins.execute(params![
                        e.task_id.as_str(),
                        e.at.0,
                        enum_to_str(&e.level)?,
                        e.code,
                        e.message,
                    ])?;
                    touched.insert(e.task_id.0.clone());
                }
            }
            let mut trim = conn.prepare_cached(
                "DELETE FROM task_log WHERE task_id = ?1 AND id < \
                 (SELECT id FROM task_log WHERE task_id = ?1 ORDER BY id DESC LIMIT 1 OFFSET ?2)",
            )?;
            for id in touched {
                trim.execute(params![id, TASK_LOG_KEEP - 1])?;
            }
            Ok(())
        })
    }

    /// The most recent `limit` log lines of a task in chronological order (`0` = all).
    pub async fn task_log(&self, task_id: &TaskId, limit: u32) -> StoreResult<Vec<TaskLogEntry>> {
        let id = task_id.clone();
        self.read(move |conn| {
            let limit = if limit == 0 { -1 } else { i64::from(limit) };
            let mut stmt = conn.prepare_cached(
                "SELECT task_id, at, level, code, message FROM (\
                   SELECT id, task_id, at, level, code, message FROM task_log \
                   WHERE task_id = ?1 ORDER BY id DESC LIMIT ?2) ORDER BY id ASC",
            )?;
            let rows = stmt.query_map(params![id.as_str(), limit], |r| {
                let level: String = r.get("level")?;
                Ok((
                    TaskId(r.get("task_id")?),
                    Millis(r.get("at")?),
                    level,
                    r.get::<_, String>("code")?,
                    r.get::<_, String>("message")?,
                ))
            })?;
            let mut out = Vec::new();
            for r in rows {
                let (task_id, at, level, code, message) = r?;
                out.push(TaskLogEntry {
                    task_id,
                    at,
                    level: enum_from_str::<LogLevel>("level", &level)?,
                    code,
                    message,
                });
            }
            Ok(out)
        })
        .await
    }

    /// Store raw `.torrent` bytes keyed by info hash (lower-case hex).
    pub async fn put_torrent_blob(&self, info_hash: &str, bytes: Vec<u8>) -> StoreResult<()> {
        let hash = info_hash.to_ascii_lowercase();
        self.write("put_torrent_blob", move |conn| {
            conn.prepare_cached(
                "INSERT INTO torrent_blobs (info_hash, bytes, added_at) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(info_hash) DO UPDATE SET bytes = excluded.bytes",
            )?
            .execute(params![hash, bytes, Millis::now().0])?;
            Ok(())
        })
        .await
    }

    /// Fetch raw `.torrent` bytes.
    pub async fn get_torrent_blob(&self, info_hash: &str) -> StoreResult<Option<Vec<u8>>> {
        let hash = info_hash.to_ascii_lowercase();
        self.read(move |conn| {
            Ok(conn
                .prepare_cached("SELECT bytes FROM torrent_blobs WHERE info_hash = ?1")?
                .query_row([hash], |r| r.get(0))
                .optional()?)
        })
        .await
    }

    /// Delete a stored `.torrent`; returns whether it existed.
    pub async fn delete_torrent_blob(&self, info_hash: &str) -> StoreResult<bool> {
        let hash = info_hash.to_ascii_lowercase();
        self.write("delete_torrent_blob", move |conn| {
            let n = conn
                .prepare_cached("DELETE FROM torrent_blobs WHERE info_hash = ?1")?
                .execute([hash])?;
            Ok(n > 0)
        })
        .await
    }

    /// The completion transaction (spec invariant 3): rewrite the task row (state, file path,
    /// verified checksum, …), write final progress, delete the checkpoint and insert the
    /// history entry — all or nothing.
    pub async fn complete_task(&self, task: &Task, history: &HistoryEntry) -> StoreResult<()> {
        let (task, history) = (task.clone(), history.clone());
        self.write("complete_task", move |conn| {
            update_cold_row(conn, &task)?;
            upsert_progress_row(conn, &task.id, &task.progress, Millis::now())?;
            delete_checkpoint_row(conn, &task.id)?;
            crate::history::insert_history_row(conn, &history)?;
            Ok(())
        })
        .await
    }
}
