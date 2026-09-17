//! Forward-only schema migrations keyed on `PRAGMA user_version`.
//!
//! Each migration is one embedded `.sql` file applied inside a single transaction; a failure
//! rolls the whole step back and leaves `user_version` untouched, so a crash mid-migration is
//! simply retried on the next start. To add a migration see the crate README.

use crate::error::{StoreError, StoreResult};
use rusqlite::Connection;

/// Ordered list of `(target_version, sql)`. Versions must be contiguous and start at 1.
const MIGRATIONS: &[(u32, &str)] = &[(1, include_str!("../migrations/v1.sql"))];

/// The schema version produced by applying every migration.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Read `PRAGMA user_version`.
pub fn schema_version(conn: &Connection) -> StoreResult<u32> {
    let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    Ok(u32::try_from(v).unwrap_or(0))
}

/// Apply every migration newer than the database's `user_version`. Refuses databases written
/// by a newer build.
pub fn apply(conn: &mut Connection) -> StoreResult<u32> {
    let mut current = schema_version(conn)?;
    if current > CURRENT_SCHEMA_VERSION {
        return Err(StoreError::SchemaTooNew {
            found: current,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }
    for (version, sql) in MIGRATIONS {
        if *version <= current {
            continue;
        }
        tracing::info!(from = current, to = version, "applying store migration");
        let tx = conn.transaction()?;
        tx.execute_batch(sql).map_err(|source| StoreError::Migration {
            version: *version,
            source,
        })?;
        tx.pragma_update(None, "user_version", *version)
            .map_err(|source| StoreError::Migration {
                version: *version,
                source,
            })?;
        tx.commit().map_err(|source| StoreError::Migration {
            version: *version,
            source,
        })?;
        current = *version;
    }
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_contiguous() {
        for (i, (v, _)) in MIGRATIONS.iter().enumerate() {
            assert_eq!(*v as usize, i + 1);
        }
        assert_eq!(
            MIGRATIONS.last().map(|m| m.0),
            Some(CURRENT_SCHEMA_VERSION)
        );
    }

    #[test]
    fn empty_database_migrates_to_current() {
        let mut conn = Connection::open_in_memory().expect("open");
        assert_eq!(apply(&mut conn).expect("apply"), CURRENT_SCHEMA_VERSION);
        assert_eq!(schema_version(&conn).expect("v"), CURRENT_SCHEMA_VERSION);
        // idempotent
        assert_eq!(apply(&mut conn).expect("apply"), CURRENT_SCHEMA_VERSION);
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .expect("prepare")
            .query_map([], |r| r.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows");
        for t in [
            "tasks",
            "task_progress",
            "task_checkpoints",
            "task_log",
            "torrent_blobs",
            "history",
            "queues",
            "categories",
            "rules",
            "schedules",
            "automations",
            "automation_runs",
            "consents",
            "recipes",
            "devices",
            "audit",
            "settings",
            "credentials_index",
            "grabber_sessions",
            "speed_samples",
            "meta",
        ] {
            assert!(tables.iter().any(|x| x == t), "missing table {t}");
        }
    }

    #[test]
    fn newer_schema_is_refused() {
        let mut conn = Connection::open_in_memory().expect("open");
        conn.pragma_update(None, "user_version", 99).expect("pragma");
        assert!(matches!(
            apply(&mut conn),
            Err(StoreError::SchemaTooNew {
                found: 99,
                supported: CURRENT_SCHEMA_VERSION
            })
        ));
    }
}
