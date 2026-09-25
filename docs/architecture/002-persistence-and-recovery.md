# Persistence and crash recovery

## Invariants

1. **Committed watermark.** For segmented downloads, `Segment.committed` is the first byte not
   yet on stable storage. It advances only after `FileWriter::flush(Barrier)` returned `Ok`.
   Everything from `committed` onward is fetched again after a restart.
2. **Checkpoint after flush.** Engines call `ProgressSink::checkpoint` only after the flush that
   covers the data. The services layer persists checkpoints debounced (≤ every 3 s) and
   immediately on pause/stop/shutdown. Persisting an older checkpoint is always safe.
3. **Completion is transactional.** `.swoop-part` → final rename (after `flush(Full)`), then in
   one SQLite transaction: task state `Completed`, `file_path`, `verified_checksum`, history row,
   checkpoint row deleted.
4. **Single writer.** One thread owns the SQLite connection (WAL, `synchronous=NORMAL`,
   `busy_timeout=5000`, `foreign_keys=ON`, `wal_autocheckpoint=1000`, `mmap_size=64MB`).
   Operations arrive over an mpsc channel; each 250 ms tick applies the batch in one
   `BEGIN IMMEDIATE`. `PRAGMA wal_checkpoint(TRUNCATE)` runs on idle (≥ 30 s no writes) and at
   shutdown.
5. **Single instance.** `swoop.lock` (flock) is taken before the database is opened.

## Synchronous vs debounced writes

| Operation | Mode |
|---|---|
| task add / remove, transitions into Paused, Completed, Failed, Cancelled, Scheduled | synchronous (caller awaits ack) |
| settings, queues, categories, rules, schedules, automations, consents, devices, tokens | synchronous |
| shutdown flush | synchronous |
| progress counters (`task_progress`) | debounced, latest-wins per task, 2 s |
| checkpoints (`task_checkpoints`) | debounced 3 s; immediate on pause/stop/shutdown |
| stats / health / task log lines | debounced 2 s; log rows appended in batches |
| torrent uploaded/ratio | debounced 10 s |

## Schema (hot/cold split)

- `tasks` — cold columns (`id PK, rev, kind, state, queue_id, category_id, schedule_id, priority,
  position, name, directory, file_path, origin, mime, created_at, updated_at, started_at,
  completed_at, attempt, next_retry_at, name_locked, directory_locked, source_json, options_json,
  error_json, stats_json, health_json, media_json, torrent_json, tags_json, blocked_json,
  verified_checksum_json`). Indexes: `(state)`, `(queue_id, state, position)`, partial
  `(next_retry_at) WHERE state='retrying'`, `(created_at)`.
- `task_progress(task_id PK REFERENCES tasks ON DELETE CASCADE, downloaded, uploaded, total, updated_at)` — hot.
- `task_checkpoints(task_id PK REFERENCES tasks ON DELETE CASCADE, kind, blob, etag, last_modified, total, part_path, updated_at)` — hot, UPSERT.
- `task_log(id PK, task_id, at, level, code, message)`, index `(task_id, at)`, trimmed to 500 rows/task.
- `torrent_blobs(info_hash PK, bytes, added_at)` — we own the `.torrent`, not only librqbit's session.
- `history(...)` with FTS5 virtual table over `name, original_url, domain, tags`.
- `queues`, `categories`, `rules`, `schedules`, `automations`, `automation_runs`, `recipes`,
  `devices(id, name, kind, scopes_json, token_hash, created_at, last_seen_at, last_ip, expires_at, revoked)`,
  `audit`, `consents(automation_id, action_index, hash, granted_at, PK(automation_id, action_index))`,
  `settings(id=1, json, schema_version)`, `credentials_index(id, name, username, created_at)` (secret in keychain),
  `grabber_sessions`, `speed_samples(at, download, upload)` (ring of 24 h at 10 s), `meta(key, value)`.
- Migrations: `PRAGMA user_version`, one `.sql`/fn per version, applied in a transaction, forward-only.

## Recovery table (applied at startup, before engines start)

| Persisted state | Part file | Action |
|---|---|---|
| Pending | — | keep Pending |
| Queued / Scheduled | any | keep; scheduler re-evaluates |
| Resolving / Connecting / Downloading / Retrying | present | → `Queued` (auto-resumes when a slot frees), checkpoint kept, `blocked_by` cleared of `Shutdown` |
| Resolving / Connecting / Downloading / Retrying | missing | → `Queued`, checkpoint discarded (start over) |
| Paused | any | keep Paused, `blocked_by` preserved |
| Verifying | part present | → `Verifying` again (re-hash) |
| Verifying / Processing | part missing, final file present with `total` bytes | → run completion transaction (treat as complete) |
| Processing | part dir present (HLS) | → `Processing` again (merge is idempotent) |
| Completed / Failed / Cancelled | — | keep |
| Seeding | — | → `Resolving` (re-add to librqbit session, then `Seeding`) |
| Torrent in any active state | librqbit session entry missing | re-add from `torrent_blobs` paused → hash-check → resume |

After the table is applied, `EngineStarted` is published and the queue scheduler admits tasks in
`(queue.priority DESC, task.priority DESC, task.position ASC)` order.

## Files on disk

- `<dir>/<name>.swoop-part` — single-file downloads (HTTP/FTP/mirrors).
- `<dir>/<name>.swoop-part/` — directory for HLS segments (`init.bin`, `seg-00000.ts`, …).
- Torrents: librqbit writes directly into `<dir>/<torrent name>/`; session state under
  `data_dir/torrents/`.
- Quarantine attribute is applied to the part file before the final rename (macOS).
