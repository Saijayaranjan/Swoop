-- Osprey store schema, version 1.
-- Conventions: ids are TEXT (UUID strings), timestamps are INTEGER unix milliseconds,
-- structured data is JSON in TEXT columns, binary payloads are BLOB. Tables are STRICT so a
-- programming error binding the wrong type fails loudly instead of being coerced silently.
-- This file runs inside one transaction; do not add BEGIN/COMMIT.

------------------------------------------------------------------------------------------------
-- Tasks: cold columns (change rarely) -----------------------------------------------------------
------------------------------------------------------------------------------------------------
CREATE TABLE tasks (
    id                     TEXT    PRIMARY KEY NOT NULL,
    rev                    INTEGER NOT NULL DEFAULT 0,
    kind                   TEXT    NOT NULL,
    state                  TEXT    NOT NULL,
    queue_id               TEXT    NOT NULL,
    category_id            TEXT,
    schedule_id            TEXT,
    priority               INTEGER NOT NULL DEFAULT 1,
    position               INTEGER NOT NULL DEFAULT 0,
    name                   TEXT    NOT NULL,
    directory              TEXT    NOT NULL,
    file_path              TEXT,
    origin                 TEXT    NOT NULL DEFAULT '',
    mime                   TEXT,
    -- derived from source_json so filters do not have to parse JSON
    url                    TEXT,
    domain                 TEXT,
    created_at             INTEGER NOT NULL,
    updated_at             INTEGER NOT NULL,
    started_at             INTEGER,
    completed_at           INTEGER,
    attempt                INTEGER NOT NULL DEFAULT 0,
    next_retry_at          INTEGER,
    name_locked            INTEGER NOT NULL DEFAULT 0,
    directory_locked       INTEGER NOT NULL DEFAULT 0,
    status_detail          TEXT,
    source_json            TEXT    NOT NULL,
    options_json           TEXT    NOT NULL,
    error_json             TEXT,
    stats_json             TEXT    NOT NULL,
    health_json            TEXT    NOT NULL,
    media_json             TEXT,
    torrent_json           TEXT,
    tags_json              TEXT    NOT NULL DEFAULT '[]',
    blocked_json           TEXT    NOT NULL DEFAULT '[]',
    verified_checksum_json TEXT
) STRICT;

CREATE INDEX tasks_state ON tasks(state);
CREATE INDEX tasks_queue_state_position ON tasks(queue_id, state, position);
CREATE INDEX tasks_next_retry ON tasks(next_retry_at) WHERE state = 'retrying';
CREATE INDEX tasks_created ON tasks(created_at);
CREATE INDEX tasks_domain ON tasks(domain);

------------------------------------------------------------------------------------------------
-- Tasks: hot rows (written at high frequency, kept apart so cold rows never churn) --------------
------------------------------------------------------------------------------------------------
CREATE TABLE task_progress (
    task_id    TEXT    PRIMARY KEY NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    downloaded INTEGER NOT NULL DEFAULT 0,
    uploaded   INTEGER NOT NULL DEFAULT 0,
    total      INTEGER,
    fraction   REAL    NOT NULL DEFAULT 0,
    ratio      REAL    NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE task_checkpoints (
    task_id       TEXT    PRIMARY KEY NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    kind          TEXT    NOT NULL,
    blob          BLOB    NOT NULL,
    etag          TEXT,
    last_modified TEXT,
    total         INTEGER,
    part_path     TEXT,
    updated_at    INTEGER NOT NULL
) STRICT;

CREATE TABLE task_log (
    id      INTEGER PRIMARY KEY,
    task_id TEXT    NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    at      INTEGER NOT NULL,
    level   TEXT    NOT NULL,
    code    TEXT    NOT NULL,
    message TEXT    NOT NULL
) STRICT;

CREATE INDEX task_log_task_at ON task_log(task_id, at);

CREATE TABLE torrent_blobs (
    info_hash TEXT    PRIMARY KEY NOT NULL,
    bytes     BLOB    NOT NULL,
    added_at  INTEGER NOT NULL
) STRICT;

------------------------------------------------------------------------------------------------
-- History (survives "clear list") + FTS5 --------------------------------------------------------
------------------------------------------------------------------------------------------------
-- `id INTEGER PRIMARY KEY` gives a stable rowid for the external-content FTS table (rowids of
-- tables without an INTEGER PRIMARY KEY may change on VACUUM).
CREATE TABLE history (
    id               INTEGER PRIMARY KEY,
    task_id          TEXT    NOT NULL UNIQUE,
    kind             TEXT    NOT NULL,
    name             TEXT    NOT NULL CHECK (name <> ''),
    original_url     TEXT    NOT NULL,
    final_url        TEXT,
    domain           TEXT    NOT NULL,
    size             INTEGER,
    checksum_algo    TEXT,
    checksum_value   TEXT,
    state            TEXT    NOT NULL,
    destination      TEXT    NOT NULL,
    category_id      TEXT,
    queue_id         TEXT    NOT NULL,
    started_at       INTEGER,
    finished_at      INTEGER NOT NULL,
    duration_seconds INTEGER NOT NULL DEFAULT 0,
    average_speed    INTEGER NOT NULL DEFAULT 0,
    peak_speed       INTEGER NOT NULL DEFAULT 0,
    error            TEXT,
    -- exact round-trip form and a space-joined form for the tokenizer
    tags_json        TEXT    NOT NULL DEFAULT '[]',
    tags             TEXT    NOT NULL DEFAULT '',
    origin           TEXT    NOT NULL DEFAULT ''
) STRICT;

CREATE INDEX history_finished ON history(finished_at);
CREATE INDEX history_domain ON history(domain);
CREATE INDEX history_checksum ON history(checksum_algo, checksum_value);
CREATE INDEX history_original_url ON history(original_url);
CREATE INDEX history_final_url ON history(final_url);
CREATE INDEX history_name_size ON history(name, size);

CREATE VIRTUAL TABLE history_fts USING fts5(
    name, original_url, domain, tags,
    content = 'history',
    content_rowid = 'id',
    tokenize = 'unicode61'
);

CREATE TRIGGER history_ai AFTER INSERT ON history BEGIN
    INSERT INTO history_fts(rowid, name, original_url, domain, tags)
    VALUES (new.id, new.name, new.original_url, new.domain, new.tags);
END;

CREATE TRIGGER history_ad AFTER DELETE ON history BEGIN
    INSERT INTO history_fts(history_fts, rowid, name, original_url, domain, tags)
    VALUES ('delete', old.id, old.name, old.original_url, old.domain, old.tags);
END;

CREATE TRIGGER history_au AFTER UPDATE ON history BEGIN
    INSERT INTO history_fts(history_fts, rowid, name, original_url, domain, tags)
    VALUES ('delete', old.id, old.name, old.original_url, old.domain, old.tags);
    INSERT INTO history_fts(rowid, name, original_url, domain, tags)
    VALUES (new.id, new.name, new.original_url, new.domain, new.tags);
END;

------------------------------------------------------------------------------------------------
-- Configuration entities: JSON documents plus the columns needed for ordering/lookup ------------
------------------------------------------------------------------------------------------------
CREATE TABLE queues (
    id         TEXT    PRIMARY KEY NOT NULL,
    name       TEXT    NOT NULL,
    position   INTEGER NOT NULL DEFAULT 0,
    json       TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE categories (
    id         TEXT    PRIMARY KEY NOT NULL,
    name       TEXT    NOT NULL,
    position   INTEGER NOT NULL DEFAULT 0,
    json       TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE rules (
    id         TEXT    PRIMARY KEY NOT NULL,
    name       TEXT    NOT NULL,
    priority   INTEGER NOT NULL DEFAULT 100,
    enabled    INTEGER NOT NULL DEFAULT 1,
    json       TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE schedules (
    id         TEXT    PRIMARY KEY NOT NULL,
    name       TEXT    NOT NULL,
    enabled    INTEGER NOT NULL DEFAULT 1,
    json       TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE automations (
    id         TEXT    PRIMARY KEY NOT NULL,
    name       TEXT    NOT NULL,
    enabled    INTEGER NOT NULL DEFAULT 1,
    json       TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE automation_runs (
    id            INTEGER PRIMARY KEY,
    automation_id TEXT    NOT NULL,
    task_id       TEXT,
    event         TEXT    NOT NULL,
    at            INTEGER NOT NULL,
    success       INTEGER NOT NULL,
    message       TEXT    NOT NULL DEFAULT ''
) STRICT;

CREATE INDEX automation_runs_automation_at ON automation_runs(automation_id, at);

CREATE TABLE consents (
    automation_id TEXT    NOT NULL REFERENCES automations(id) ON DELETE CASCADE,
    action_index  INTEGER NOT NULL,
    hash          TEXT    NOT NULL,
    granted_at    INTEGER NOT NULL,
    PRIMARY KEY (automation_id, action_index)
) STRICT;

CREATE TABLE recipes (
    id         TEXT    PRIMARY KEY NOT NULL,
    name       TEXT    NOT NULL,
    json       TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

------------------------------------------------------------------------------------------------
-- Remote devices + audit -----------------------------------------------------------------------
------------------------------------------------------------------------------------------------
CREATE TABLE devices (
    id           TEXT    PRIMARY KEY NOT NULL,
    name         TEXT    NOT NULL,
    kind         TEXT    NOT NULL,
    scopes_json  TEXT    NOT NULL DEFAULT '[]',
    token_hash   TEXT,
    created_at   INTEGER NOT NULL,
    last_seen_at INTEGER,
    last_ip      TEXT,
    expires_at   INTEGER,
    revoked      INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE UNIQUE INDEX devices_token_hash ON devices(token_hash) WHERE token_hash IS NOT NULL;

CREATE TABLE audit (
    id        INTEGER PRIMARY KEY,
    at        INTEGER NOT NULL,
    device_id TEXT,
    ip        TEXT    NOT NULL,
    action    TEXT    NOT NULL,
    target    TEXT,
    success   INTEGER NOT NULL,
    detail    TEXT
) STRICT;

CREATE INDEX audit_at ON audit(at);

------------------------------------------------------------------------------------------------
-- Settings, credentials index, grabber sessions, speed samples, meta ----------------------------
------------------------------------------------------------------------------------------------
CREATE TABLE settings (
    id             INTEGER PRIMARY KEY CHECK (id = 1),
    json           TEXT    NOT NULL,
    schema_version INTEGER NOT NULL
) STRICT;

CREATE TABLE credentials_index (
    id         TEXT    PRIMARY KEY NOT NULL,
    name       TEXT    NOT NULL,
    username   TEXT,
    created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE grabber_sessions (
    id         TEXT    PRIMARY KEY NOT NULL,
    json       TEXT    NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE speed_samples (
    at       INTEGER PRIMARY KEY,
    download INTEGER NOT NULL,
    upload   INTEGER NOT NULL
) STRICT;

CREATE TABLE meta (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;
