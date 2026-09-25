//! Read path and row-mapping helpers.
//!
//! Reads use a second, read-only connection (WAL allows readers alongside the writer). It is
//! guarded by a mutex and every read runs on Tokio's blocking pool so async callers never block
//! a worker thread on SQLite.

use crate::error::{StoreError, StoreResult};
use parking_lot::Mutex;
use rusqlite::types::Value;
use rusqlite::{Connection, OpenFlags, Row};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Shared handle to the read-only connection. `None` once the store is closed.
pub(crate) type ReaderConn = Arc<Mutex<Option<Connection>>>;

/// Open the read-only connection and apply its pragmas.
pub(crate) fn open_reader(path: &Path) -> StoreResult<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.execute_batch(
        "PRAGMA busy_timeout = 5000;
         PRAGMA mmap_size = 67108864;
         PRAGMA query_only = ON;
         PRAGMA cache_size = -8000;",
    )?;
    Ok(conn)
}

/// Run `f` against the read-only connection on the blocking pool.
pub(crate) async fn read<T, F>(reader: &ReaderConn, f: F) -> StoreResult<T>
where
    T: Send + 'static,
    F: FnOnce(&Connection) -> StoreResult<T> + Send + 'static,
{
    let reader = Arc::clone(reader);
    tokio::task::spawn_blocking(move || {
        let guard = reader.lock();
        match guard.as_ref() {
            Some(conn) => f(conn),
            None => Err(StoreError::Closed),
        }
    })
    .await
    .map_err(|e| StoreError::Internal(format!("read task failed: {e}")))?
}

// ---------------------------------------------------------------------------------------------
// JSON helpers
// ---------------------------------------------------------------------------------------------

/// Serialise a value for a JSON TEXT column.
pub(crate) fn to_json<T: Serialize>(v: &T) -> StoreResult<String> {
    Ok(serde_json::to_string(v)?)
}

/// Serialise an optional value; `None` becomes SQL NULL.
pub(crate) fn to_json_opt<T: Serialize>(v: &Option<T>) -> StoreResult<Option<String>> {
    v.as_ref().map(to_json).transpose()
}

/// Deserialise a JSON TEXT column, naming the column in the error.
pub(crate) fn from_json<T: DeserializeOwned>(column: &str, s: &str) -> StoreResult<T> {
    serde_json::from_str(s).map_err(|e| StoreError::Corrupt(format!("{column}: {e}")))
}

/// Deserialise an optional JSON TEXT column.
pub(crate) fn from_json_opt<T: DeserializeOwned>(
    column: &str,
    s: Option<String>,
) -> StoreResult<Option<T>> {
    s.as_deref().map(|s| from_json(column, s)).transpose()
}

/// The plain string form of a unit enum variant that serialises as a string
/// (`"download_completed"`, `"info"`, …).
pub(crate) fn enum_to_str<T: Serialize>(v: &T) -> StoreResult<String> {
    match serde_json::to_value(v)? {
        serde_json::Value::String(s) => Ok(s),
        other => Err(StoreError::Internal(format!(
            "expected string-like enum, got {other}"
        ))),
    }
}

/// Inverse of [`enum_to_str`].
pub(crate) fn enum_from_str<T: DeserializeOwned>(column: &str, s: &str) -> StoreResult<T> {
    serde_json::from_value(serde_json::Value::String(s.to_owned()))
        .map_err(|e| StoreError::Corrupt(format!("{column}: {e}")))
}

// ---------------------------------------------------------------------------------------------
// Path / scalar helpers
// ---------------------------------------------------------------------------------------------

/// Paths are stored as UTF-8 text (lossy for the rare non-UTF-8 name).
pub(crate) fn path_to_text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Optional path column.
pub(crate) fn path_opt_to_text(p: &Option<PathBuf>) -> Option<String> {
    p.as_deref().map(path_to_text)
}

/// Read a `u64` from an INTEGER column (SQLite stores signed 64-bit; negatives are corrupt).
pub(crate) fn get_u64(row: &Row<'_>, column: &str) -> StoreResult<u64> {
    let v: i64 = row.get(column)?;
    u64::try_from(v).map_err(|_| StoreError::Corrupt(format!("{column}: negative value {v}")))
}

/// Read an optional `u64`.
pub(crate) fn get_u64_opt(row: &Row<'_>, column: &str) -> StoreResult<Option<u64>> {
    let v: Option<i64> = row.get(column)?;
    v.map(|v| {
        u64::try_from(v).map_err(|_| StoreError::Corrupt(format!("{column}: negative value {v}")))
    })
    .transpose()
}

/// Bind a `u64` (SQLite integers are signed; values above `i64::MAX` are clamped).
pub(crate) fn u64_to_sql(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

/// Bind an optional `u64`.
pub(crate) fn u64_opt_to_sql(v: Option<u64>) -> Option<i64> {
    v.map(u64_to_sql)
}

/// Escape a user string for `LIKE ? ESCAPE '\'` and wrap it in wildcards.
pub(crate) fn like_contains(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('%');
    for c in s.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// Turn free text into an FTS5 query: every whitespace-separated token becomes a quoted prefix
/// term, so punctuation in the input can never break the query syntax.
pub(crate) fn fts_query(text: &str) -> Option<String> {
    let terms: Vec<String> = text
        .split_whitespace()
        .map(|t| t.trim_matches('"'))
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"*", t.replace('"', "\"\"")))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" "))
    }
}

/// Dynamic parameter list for filter queries.
#[derive(Default)]
pub(crate) struct Params(pub Vec<Value>);

impl Params {
    pub fn push<V: Into<Value>>(&mut self, v: V) -> &mut Self {
        self.0.push(v.into());
        self
    }
    pub fn text(&mut self, s: impl Into<String>) -> &mut Self {
        self.push(Value::Text(s.into()))
    }
    pub fn int(&mut self, v: i64) -> &mut Self {
        self.push(Value::Integer(v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_escaping() {
        assert_eq!(like_contains("a%b_c\\"), "%a\\%b\\_c\\\\%");
    }

    #[test]
    fn fts_query_quotes_tokens() {
        assert_eq!(
            fts_query("ubuntu 24.04 \"iso\"").as_deref(),
            Some("\"ubuntu\"* \"24.04\"* \"iso\"*")
        );
        assert_eq!(fts_query("   "), None);
        assert_eq!(fts_query("a\"b").as_deref(), Some("\"a\"\"b\"*"));
    }

    #[test]
    fn enum_round_trip() {
        use swoop_domain::events::LogLevel;
        let s = enum_to_str(&LogLevel::Warn).expect("to");
        assert_eq!(s, "warn");
        let back: LogLevel = enum_from_str("level", &s).expect("from");
        assert_eq!(back, LogLevel::Warn);
    }
}
