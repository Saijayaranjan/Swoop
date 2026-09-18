//! Download history: one row per finished task, with an FTS5 index over name, URL, domain and
//! tags for the search box, plus the exact-match lookups duplicate detection needs.

use crate::error::{StoreError, StoreResult};
use crate::reader::{
    from_json, fts_query, get_u64, get_u64_opt, path_to_text, to_json, u64_opt_to_sql, u64_to_sql,
    Params,
};
use crate::Store;
use osprey_domain::history::{HistoryEntry, HistoryQuery, HistorySort};
use osprey_domain::{
    CategoryId, Checksum, ChecksumAlgorithm, Millis, QueueId, TaskId, TaskKind, TaskState,
};
use rusqlite::{params, params_from_iter, Connection, Row};

const HISTORY_COLUMNS: &str = "task_id, kind, name, original_url, final_url, domain, size, \
    checksum_algo, checksum_value, state, destination, category_id, queue_id, started_at, \
    finished_at, duration_seconds, average_speed, peak_speed, error, tags_json, origin";

fn history_from_row(row: &Row<'_>) -> StoreResult<HistoryEntry> {
    let kind_s: String = row.get("kind")?;
    let state_s: String = row.get("state")?;
    let tags_json: String = row.get("tags_json")?;
    let checksum = match (
        row.get::<_, Option<String>>("checksum_algo")?,
        row.get::<_, Option<String>>("checksum_value")?,
    ) {
        (Some(a), Some(v)) => Some(Checksum {
            algorithm: ChecksumAlgorithm::parse(&a)
                .ok_or_else(|| StoreError::Corrupt(format!("checksum_algo: {a}")))?,
            value: v,
        }),
        _ => None,
    };
    Ok(HistoryEntry {
        task_id: TaskId(row.get("task_id")?),
        kind: TaskKind::parse(&kind_s)
            .ok_or_else(|| StoreError::Corrupt(format!("history.kind: {kind_s}")))?,
        name: row.get("name")?,
        original_url: row.get("original_url")?,
        final_url: row.get("final_url")?,
        domain: row.get("domain")?,
        size: get_u64_opt(row, "size")?,
        checksum,
        state: TaskState::parse(&state_s)
            .ok_or_else(|| StoreError::Corrupt(format!("history.state: {state_s}")))?,
        destination: row.get::<_, String>("destination")?.into(),
        category_id: row.get::<_, Option<String>>("category_id")?.map(CategoryId),
        queue_id: QueueId(row.get("queue_id")?),
        started_at: row.get::<_, Option<i64>>("started_at")?.map(Millis),
        finished_at: Millis(row.get("finished_at")?),
        duration_seconds: get_u64(row, "duration_seconds")?,
        average_speed: get_u64(row, "average_speed")?,
        peak_speed: get_u64(row, "peak_speed")?,
        error: row.get("error")?,
        tags: from_json("tags_json", &tags_json)?,
        origin: row.get("origin")?,
    })
}

/// Insert or replace (by `task_id`) one history row. Used by `complete_task` as well.
pub(crate) fn insert_history_row(conn: &Connection, h: &HistoryEntry) -> StoreResult<()> {
    conn.prepare_cached(
        "INSERT INTO history (task_id, kind, name, original_url, final_url, domain, size, \
         checksum_algo, checksum_value, state, destination, category_id, queue_id, started_at, \
         finished_at, duration_seconds, average_speed, peak_speed, error, tags_json, tags, \
         origin) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, \
         ?17, ?18, ?19, ?20, ?21, ?22) \
         ON CONFLICT(task_id) DO UPDATE SET kind = excluded.kind, name = excluded.name, \
         original_url = excluded.original_url, final_url = excluded.final_url, \
         domain = excluded.domain, size = excluded.size, checksum_algo = excluded.checksum_algo, \
         checksum_value = excluded.checksum_value, state = excluded.state, \
         destination = excluded.destination, category_id = excluded.category_id, \
         queue_id = excluded.queue_id, started_at = excluded.started_at, \
         finished_at = excluded.finished_at, duration_seconds = excluded.duration_seconds, \
         average_speed = excluded.average_speed, peak_speed = excluded.peak_speed, \
         error = excluded.error, tags_json = excluded.tags_json, tags = excluded.tags, \
         origin = excluded.origin",
    )?
    .execute(params![
        h.task_id.as_str(),
        h.kind.as_str(),
        h.name,
        h.original_url,
        h.final_url,
        h.domain.to_lowercase(),
        u64_opt_to_sql(h.size),
        h.checksum.as_ref().map(|c| c.algorithm.as_str()),
        h.checksum.as_ref().map(|c| c.value.to_ascii_lowercase()),
        h.state.as_str(),
        path_to_text(&h.destination),
        h.category_id.as_ref().map(|c| c.as_str()),
        h.queue_id.as_str(),
        h.started_at.map(|m| m.0),
        h.finished_at.0,
        u64_to_sql(h.duration_seconds),
        u64_to_sql(h.average_speed),
        u64_to_sql(h.peak_speed),
        h.error,
        to_json(&h.tags)?,
        h.tags.join(" "),
        h.origin,
    ])?;
    Ok(())
}

fn where_clause(q: &HistoryQuery) -> (String, Params) {
    let mut conds: Vec<String> = Vec::new();
    let mut p = Params::default();
    if let Some(fts) = q.text.as_deref().and_then(fts_query) {
        conds.push("h.id IN (SELECT rowid FROM history_fts WHERE history_fts MATCH ?)".into());
        p.text(fts);
    }
    if let Some(d) = &q.domain {
        conds.push("h.domain = ?".into());
        p.text(d.to_lowercase());
    }
    if let Some(s) = q.state {
        conds.push("h.state = ?".into());
        p.text(s.as_str());
    }
    if let Some(k) = q.kind {
        conds.push("h.kind = ?".into());
        p.text(k.as_str());
    }
    if let Some(c) = &q.category_id {
        conds.push("h.category_id = ?".into());
        p.text(c.as_str());
    }
    if let Some(qid) = &q.queue_id {
        conds.push("h.queue_id = ?".into());
        p.text(qid.as_str());
    }
    if let Some(since) = q.since {
        conds.push("h.finished_at >= ?".into());
        p.int(since.0);
    }
    if let Some(until) = q.until {
        conds.push("h.finished_at < ?".into());
        p.int(until.0);
    }
    if let Some(min) = q.min_size {
        conds.push("COALESCE(h.size, 0) >= ?".into());
        p.int(u64_to_sql(min));
    }
    if let Some(max) = q.max_size {
        conds.push("COALESCE(h.size, 0) <= ?".into());
        p.int(u64_to_sql(max));
    }
    if let Some(tag) = &q.tag {
        conds.push("EXISTS (SELECT 1 FROM json_each(h.tags_json) WHERE value = ?)".into());
        p.text(tag.as_str());
    }
    let sql = if conds.is_empty() {
        "1 = 1".to_owned()
    } else {
        conds.join(" AND ")
    };
    (sql, p)
}

fn order_clause(q: &HistoryQuery) -> String {
    let dir = if q.descending { "DESC" } else { "ASC" };
    let key = match q.sort {
        HistorySort::FinishedAt => "h.finished_at",
        HistorySort::Name => "h.name COLLATE NOCASE",
        HistorySort::Size => "COALESCE(h.size, 0)",
        HistorySort::Domain => "h.domain",
        HistorySort::Duration => "h.duration_seconds",
        HistorySort::Speed => "h.average_speed",
    };
    format!("ORDER BY {key} {dir}, h.id {dir}")
}

fn query_rows(conn: &Connection, sql: &str, params: Params) -> StoreResult<Vec<HistoryEntry>> {
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(params_from_iter(params.0), |r| Ok(history_from_row(r)))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r??);
    }
    Ok(out)
}

impl Store {
    /// Insert (or replace by task id) a history entry.
    pub async fn insert_history(&self, entry: &HistoryEntry) -> StoreResult<()> {
        let entry = entry.clone();
        self.write("insert_history", move |conn| {
            insert_history_row(conn, &entry)
        })
        .await
    }

    /// Search and page the history. `text` uses the FTS5 index (prefix match per token).
    pub async fn query_history(&self, query: &HistoryQuery) -> StoreResult<Vec<HistoryEntry>> {
        let q = query.clone();
        self.read(move |conn| {
            let (where_sql, mut params) = where_clause(&q);
            let order = order_clause(&q);
            let limit = if q.limit == 0 { -1 } else { i64::from(q.limit) };
            params.int(limit).int(i64::from(q.offset));
            let sql = format!(
                "SELECT {HISTORY_COLUMNS} FROM history h WHERE {where_sql} {order} \
                 LIMIT ? OFFSET ?"
            );
            query_rows(conn, &sql, params)
        })
        .await
    }

    /// Number of history rows matching the query (paging ignored).
    pub async fn count_history(&self, query: &HistoryQuery) -> StoreResult<u64> {
        let q = query.clone();
        self.read(move |conn| {
            let (where_sql, params) = where_clause(&q);
            let sql = format!("SELECT COUNT(*) FROM history h WHERE {where_sql}");
            let n: i64 = conn.query_row(&sql, params_from_iter(params.0), |r| r.get(0))?;
            Ok(n.max(0) as u64)
        })
        .await
    }

    /// Delete entries by task id; returns how many were removed.
    pub async fn delete_history(&self, ids: &[TaskId]) -> StoreResult<u64> {
        let ids: Vec<String> = ids.iter().map(|i| i.0.clone()).collect();
        self.write("delete_history", move |conn| {
            let mut stmt = conn.prepare_cached("DELETE FROM history WHERE task_id = ?1")?;
            let mut n = 0u64;
            for id in &ids {
                n += stmt.execute([id])? as u64;
            }
            Ok(n)
        })
        .await
    }

    /// Delete every history entry; returns how many were removed.
    pub async fn clear_history(&self) -> StoreResult<u64> {
        self.write("clear_history", |conn| {
            let n = conn.execute("DELETE FROM history", [])? as u64;
            Ok(n)
        })
        .await
    }

    /// Entries whose verified checksum matches (duplicate detection).
    pub async fn find_history_by_checksum(
        &self,
        algorithm: ChecksumAlgorithm,
        value: &str,
    ) -> StoreResult<Vec<HistoryEntry>> {
        let value = value.to_ascii_lowercase();
        self.read(move |conn| {
            let sql = format!(
                "SELECT {HISTORY_COLUMNS} FROM history h \
                 WHERE h.checksum_algo = ? AND h.checksum_value = ? ORDER BY h.finished_at DESC"
            );
            let mut p = Params::default();
            p.text(algorithm.as_str()).text(value);
            query_rows(conn, &sql, p)
        })
        .await
    }

    /// Entries whose original or final URL equals `url` (duplicate detection).
    pub async fn find_history_by_url(&self, url: &str) -> StoreResult<Vec<HistoryEntry>> {
        let url = url.to_owned();
        self.read(move |conn| {
            let sql = format!(
                "SELECT {HISTORY_COLUMNS} FROM history h \
                 WHERE h.original_url = ?1 OR h.final_url = ?1 ORDER BY h.finished_at DESC"
            );
            let mut p = Params::default();
            p.text(url);
            query_rows(conn, &sql, p)
        })
        .await
    }

    /// Entries with the same file name and size (duplicate detection).
    pub async fn find_history_by_name_size(
        &self,
        name: &str,
        size: u64,
    ) -> StoreResult<Vec<HistoryEntry>> {
        let name = name.to_owned();
        self.read(move |conn| {
            let sql = format!(
                "SELECT {HISTORY_COLUMNS} FROM history h \
                 WHERE h.name = ? AND h.size = ? ORDER BY h.finished_at DESC"
            );
            let mut p = Params::default();
            p.text(name).int(u64_to_sql(size));
            query_rows(conn, &sql, p)
        })
        .await
    }
}
