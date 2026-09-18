//! Settings document, credentials index (metadata only — secrets live in the keychain),
//! grabber sessions and the 24 h speed-sample ring.

use crate::error::StoreResult;
use crate::reader::{from_json, get_u64, to_json, u64_to_sql};
use crate::Store;
use osprey_domain::settings::{Settings, SETTINGS_SCHEMA_VERSION};
use osprey_domain::{CredentialId, Millis};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Speed samples older than this are pruned on append.
pub const SPEED_SAMPLE_RETENTION_MS: i64 = 24 * 3600 * 1000;

/// Credential metadata (the secret is in the OS keychain, keyed by `id`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialMeta {
    /// Keychain reference.
    pub id: CredentialId,
    /// User-visible label.
    pub name: String,
    /// Username, if the credential has one.
    pub username: Option<String>,
    /// Creation time.
    pub created_at: Millis,
}

/// One point of the global speed graph.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeedSample {
    /// Sample time.
    pub at: Millis,
    /// Download bytes/s.
    pub download: u64,
    /// Upload bytes/s.
    pub upload: u64,
}

impl Store {
    // ----- settings -----

    /// The stored settings, or `None` when nothing was saved yet (use defaults). Older
    /// documents deserialise with serde defaults for fields they lack.
    pub async fn load_settings(&self) -> StoreResult<Option<Settings>> {
        self.read(|conn| {
            let row: Option<(String, i64)> = conn
                .prepare_cached("SELECT json, schema_version FROM settings WHERE id = 1")?
                .query_row([], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?;
            let Some((json, version)) = row else {
                return Ok(None);
            };
            if version < i64::from(SETTINGS_SCHEMA_VERSION) {
                tracing::info!(
                    stored = version,
                    current = SETTINGS_SCHEMA_VERSION,
                    "settings document predates the current schema; applying defaults"
                );
            }
            let settings: Settings = from_json("settings.json", &json)?;
            Ok(Some(settings))
        })
        .await
    }

    /// Save the settings document.
    pub async fn save_settings(&self, settings: &Settings) -> StoreResult<()> {
        let s = settings.clone();
        self.write("save_settings", move |conn| {
            conn.prepare_cached(
                "INSERT INTO settings (id, json, schema_version) VALUES (1, ?1, ?2) \
                 ON CONFLICT(id) DO UPDATE SET json = excluded.json, \
                 schema_version = excluded.schema_version",
            )?
            .execute(params![to_json(&s)?, i64::from(s.schema_version)])?;
            Ok(())
        })
        .await
    }

    // ----- credentials index -----

    /// Insert or rename a credential's metadata.
    pub async fn upsert_credential_meta(
        &self,
        id: &CredentialId,
        name: &str,
        username: Option<&str>,
    ) -> StoreResult<()> {
        let (id, name, username) = (id.clone(), name.to_owned(), username.map(str::to_owned));
        self.write("upsert_credential_meta", move |conn| {
            conn.prepare_cached(
                "INSERT INTO credentials_index (id, name, username, created_at) \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name, \
                 username = excluded.username",
            )?
            .execute(params![id.as_str(), name, username, Millis::now().0])?;
            Ok(())
        })
        .await
    }

    /// All credential metadata, by name.
    pub async fn list_credential_meta(&self) -> StoreResult<Vec<CredentialMeta>> {
        self.read(|conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT id, name, username, created_at FROM credentials_index \
                 ORDER BY name COLLATE NOCASE ASC, created_at ASC",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(CredentialMeta {
                    id: CredentialId(r.get(0)?),
                    name: r.get(1)?,
                    username: r.get(2)?,
                    created_at: Millis(r.get(3)?),
                })
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .await
    }

    /// Delete credential metadata (the caller removes the keychain item).
    pub async fn delete_credential_meta(&self, id: &CredentialId) -> StoreResult<bool> {
        let id = id.clone();
        self.write("delete_credential_meta", move |conn| {
            Ok(conn
                .prepare_cached("DELETE FROM credentials_index WHERE id = ?1")?
                .execute([id.as_str()])?
                > 0)
        })
        .await
    }

    // ----- grabber sessions -----

    /// Insert or replace a grabber session document.
    pub async fn save_grabber_session(
        &self,
        id: &str,
        json: &serde_json::Value,
    ) -> StoreResult<()> {
        let (id, json) = (id.to_owned(), json.clone());
        self.write("save_grabber_session", move |conn| {
            conn.prepare_cached(
                "INSERT INTO grabber_sessions (id, json, updated_at) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(id) DO UPDATE SET json = excluded.json, \
                 updated_at = excluded.updated_at",
            )?
            .execute(params![id, to_json(&json)?, Millis::now().0])?;
            Ok(())
        })
        .await
    }

    /// All grabber sessions as `(id, document)`, most recently updated first.
    pub async fn load_grabber_sessions(&self) -> StoreResult<Vec<(String, serde_json::Value)>> {
        self.read(|conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT id, json FROM grabber_sessions ORDER BY updated_at DESC, id ASC",
            )?;
            let rows =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            let mut out = Vec::new();
            for r in rows {
                let (id, json) = r?;
                out.push((id, from_json("grabber_sessions.json", &json)?));
            }
            Ok(out)
        })
        .await
    }

    /// Delete a grabber session.
    pub async fn delete_grabber_session(&self, id: &str) -> StoreResult<bool> {
        let id = id.to_owned();
        self.write("delete_grabber_session", move |conn| {
            Ok(conn
                .prepare_cached("DELETE FROM grabber_sessions WHERE id = ?1")?
                .execute([id])?
                > 0)
        })
        .await
    }

    // ----- speed samples -----

    /// Deferred: append a speed sample and prune anything older than 24 h.
    pub async fn append_speed_sample(
        &self,
        at: Millis,
        download: u64,
        upload: u64,
    ) -> StoreResult<()> {
        self.write_deferred("append_speed_sample", move |conn| {
            conn.prepare_cached(
                "INSERT INTO speed_samples (at, download, upload) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(at) DO UPDATE SET download = excluded.download, \
                 upload = excluded.upload",
            )?
            .execute(params![at.0, u64_to_sql(download), u64_to_sql(upload)])?;
            conn.prepare_cached("DELETE FROM speed_samples WHERE at < ?1")?
                .execute([at.0 - SPEED_SAMPLE_RETENTION_MS])?;
            Ok(())
        })
    }

    /// Samples at or after `since`, oldest first.
    pub async fn speed_samples(&self, since: Millis) -> StoreResult<Vec<SpeedSample>> {
        self.read(move |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT at, download, upload FROM speed_samples WHERE at >= ?1 ORDER BY at ASC",
            )?;
            let rows = stmt.query_map([since.0], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    get_u64(r, "download"),
                    get_u64(r, "upload"),
                ))
            })?;
            let mut out = Vec::new();
            for r in rows {
                let (at, d, u) = r?;
                out.push(SpeedSample {
                    at: Millis(at),
                    download: d?,
                    upload: u?,
                });
            }
            Ok(out)
        })
        .await
    }
}

impl From<(Millis, u64, u64)> for SpeedSample {
    fn from((at, download, upload): (Millis, u64, u64)) -> Self {
        Self {
            at,
            download,
            upload,
        }
    }
}
