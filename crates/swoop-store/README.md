# swoop-store

SQLite persistence for Swoop: WAL journal, one writer thread, versioned migrations and the
startup recovery pass. It implements `docs/architecture/002-persistence-and-recovery.md`.

## Schema overview (v1)

| Table | Holds | Notes |
|---|---|---|
| `tasks` | cold task columns + JSON blobs (`source_json`, `options_json`, `stats_json`, …) | `url`/`domain` are derived from the source for filtering; indexes on `state`, `(queue_id, state, position)`, `next_retry_at WHERE state='retrying'`, `created_at`, `domain` |
| `task_progress` | `downloaded, uploaded, total, fraction, ratio` | hot row, latest-wins, cascades with the task |
| `task_checkpoints` | serialised `Checkpoint` (`blob`) + `etag`/`last_modified`/`total`/`part_path` | hot row; a `Segments` checkpoint is what `Task.segment_map` is loaded from |
| `task_log` | per-task diagnostic lines | trimmed to 500 rows per task on append |
| `torrent_blobs` | raw `.torrent` bytes by info hash | |
| `history` + `history_fts` | finished downloads; FTS5 (external content, kept in sync by triggers) over `name, original_url, domain, tags` | indexes for checksum / URL / name+size duplicate lookups |
| `queues`, `categories`, `rules`, `schedules`, `automations`, `recipes` | one JSON document per entity plus ordering columns | domain structs use `#[serde(default)]`, so new optional fields need no migration |
| `consents`, `automation_runs` | consent hashes per automation action (cascade on automation delete); run log capped at 1000 | |
| `devices`, `audit` | paired devices with a unique partial index on `token_hash`; audit capped at 5000 | |
| `settings` | single row (`id = 1`), JSON + `schema_version` | |
| `credentials_index`, `grabber_sessions`, `speed_samples`, `meta` | metadata for keychain secrets, grabber documents, 24 h speed ring, key/value | |

All tables are `STRICT`. Ids are TEXT, timestamps INTEGER unix milliseconds, JSON is TEXT,
binary is BLOB.

## Threading model

```
async caller ──(closure over mpsc)──▶ writer thread (owns RW Connection)
     │                                   BEGIN IMMEDIATE … SAVEPOINT per op … COMMIT
     │                                   every ~250 ms, or at once when a caller awaits
     └──(spawn_blocking)──▶ read-only Connection behind a mutex (WAL ⇒ concurrent with writer)
```

* **Synchronous methods** (`insert_task`, `update_task_state`, `complete_task`, all
  config/device/settings writes, `upsert_checkpoint`, …) await the commit and return its result.
  A failing op is rolled back to its savepoint and reported to its caller; the rest of the batch
  still commits.
* **Deferred methods** (`upsert_progress`, `append_task_log`, `append_speed_sample`) return once
  queued; failures are logged. `flush()` waits for everything queued so far; `pending_ops()` is
  the queue depth.
* `progress_writer()` returns a `ProgressBatcher` that coalesces progress per task and writes
  latest-wins rows every 2 s.
* `close()` commits what is queued, runs `PRAGMA wal_checkpoint(TRUNCATE)`, joins the thread and
  closes both connections. The writer also checkpoints after 30 s of idleness.
* `recover(temp_suffix)` runs the recovery table before engines start and returns a
  `RecoveryReport` (including the post-recovery task snapshot and the tasks whose completion
  transaction the services layer must run).

`Store::open_in_memory()` uses a private temp directory rather than `:memory:` so tests exercise
exactly the production WAL/reader/writer behaviour.

## Adding a migration

1. Create `migrations/v<N>.sql` with plain DDL/DML. Do **not** include `BEGIN`/`COMMIT`; the
   runner wraps the file in one transaction and sets `PRAGMA user_version = N` on success.
2. Append `(N, include_str!("../migrations/v<N>.sql"))` to `MIGRATIONS` in
   `src/migrations.rs` and bump `CURRENT_SCHEMA_VERSION`.
3. Update the row mapping in the relevant module (`tasks.rs`, `history.rs`, …) and the schema
   table above.
4. `cargo test -p swoop-store` — `migrations_are_contiguous` and
   `empty_database_migrates_to_current` guard the chain; add a test that opens a v(N-1) fixture
   if the migration transforms data.

Migrations are forward-only. A database with a `user_version` newer than this build is refused
(`StoreError::SchemaTooNew`) rather than opened.
