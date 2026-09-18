//! Paired remote devices (with their bearer-token hashes) and the remote-API audit log.
//! Only hashes are stored; the plaintext token is shown to the client once and never persisted.

use crate::error::StoreResult;
use crate::reader::{from_json, to_json};
use crate::Store;
use osprey_domain::device::{AuditEntry, Device, Scope};
use osprey_domain::{DeviceId, Millis};
use rusqlite::{params, Connection, OptionalExtension, Row};

/// Audit rows kept.
pub const AUDIT_KEEP: i64 = 5000;

const DEVICE_COLUMNS: &str =
    "id, name, kind, scopes_json, created_at, last_seen_at, last_ip, expires_at, revoked";

fn device_from_row(row: &Row<'_>) -> StoreResult<Device> {
    let scopes_json: String = row.get("scopes_json")?;
    Ok(Device {
        id: DeviceId(row.get("id")?),
        name: row.get("name")?,
        kind: row.get("kind")?,
        scopes: from_json::<Vec<Scope>>("scopes_json", &scopes_json)?,
        created_at: Millis(row.get("created_at")?),
        last_seen_at: row.get::<_, Option<i64>>("last_seen_at")?.map(Millis),
        last_ip: row.get("last_ip")?,
        expires_at: row.get::<_, Option<i64>>("expires_at")?.map(Millis),
        revoked: row.get("revoked")?,
    })
}

fn query_devices<P: rusqlite::Params>(
    conn: &Connection,
    sql: &str,
    params: P,
) -> StoreResult<Vec<Device>> {
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(params, |r| Ok(device_from_row(r)))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r??);
    }
    Ok(out)
}

impl Store {
    /// Insert or replace a device. `token_hash = None` keeps the stored hash (renames and
    /// scope edits do not invalidate the token); `Some` replaces it.
    pub async fn upsert_device(
        &self,
        device: &Device,
        token_hash: Option<String>,
    ) -> StoreResult<()> {
        let d = device.clone();
        self.write("upsert_device", move |conn| {
            conn.prepare_cached(
                "INSERT INTO devices (id, name, kind, scopes_json, token_hash, created_at, \
                 last_seen_at, last_ip, expires_at, revoked) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name, kind = excluded.kind, \
                 scopes_json = excluded.scopes_json, \
                 token_hash = COALESCE(excluded.token_hash, devices.token_hash), \
                 last_seen_at = excluded.last_seen_at, last_ip = excluded.last_ip, \
                 expires_at = excluded.expires_at, revoked = excluded.revoked",
            )?
            .execute(params![
                d.id.as_str(),
                d.name,
                d.kind,
                to_json(&d.scopes)?,
                token_hash,
                d.created_at.0,
                d.last_seen_at.map(|m| m.0),
                d.last_ip,
                d.expires_at.map(|m| m.0),
                d.revoked,
            ])?;
            Ok(())
        })
        .await
    }

    /// All devices, oldest first.
    pub async fn list_devices(&self) -> StoreResult<Vec<Device>> {
        self.read(|conn| {
            query_devices(
                conn,
                &format!("SELECT {DEVICE_COLUMNS} FROM devices ORDER BY created_at ASC"),
                [],
            )
        })
        .await
    }

    /// Look a device up by the hash of its bearer token (revoked devices are returned too so
    /// the caller can audit the attempt).
    pub async fn find_device_by_token_hash(&self, token_hash: &str) -> StoreResult<Option<Device>> {
        let hash = token_hash.to_owned();
        self.read(move |conn| {
            let sql = format!("SELECT {DEVICE_COLUMNS} FROM devices WHERE token_hash = ?1");
            let row = conn
                .prepare_cached(&sql)?
                .query_row([hash], |r| Ok(device_from_row(r)))
                .optional()?;
            row.transpose()
        })
        .await
    }

    /// Record a successful authentication.
    pub async fn touch_device(&self, id: &DeviceId, ip: &str, at: Millis) -> StoreResult<()> {
        let (id, ip) = (id.clone(), ip.to_owned());
        self.write("touch_device", move |conn| {
            conn.prepare_cached(
                "UPDATE devices SET last_seen_at = ?2, last_ip = ?3 WHERE id = ?1",
            )?
            .execute(params![id.as_str(), at.0, ip])?;
            Ok(())
        })
        .await
    }

    /// Delete a device (its token stops working immediately).
    pub async fn delete_device(&self, id: &DeviceId) -> StoreResult<bool> {
        let id = id.clone();
        self.write("delete_device", move |conn| {
            Ok(conn
                .prepare_cached("DELETE FROM devices WHERE id = ?1")?
                .execute([id.as_str()])?
                > 0)
        })
        .await
    }

    /// Append an audit row, keeping the newest [`AUDIT_KEEP`].
    pub async fn append_audit(&self, entry: &AuditEntry) -> StoreResult<()> {
        let e = entry.clone();
        self.write("append_audit", move |conn| {
            conn.prepare_cached(
                "INSERT INTO audit (at, device_id, ip, action, target, success, detail) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?
            .execute(params![
                e.at.0,
                e.device_id.as_ref().map(|d| d.as_str()),
                e.ip,
                e.action,
                e.target,
                e.success,
                e.detail,
            ])?;
            conn.prepare_cached(
                "DELETE FROM audit WHERE id < (SELECT id FROM audit ORDER BY id DESC LIMIT 1 OFFSET ?1)",
            )?
            .execute([AUDIT_KEEP - 1])?;
            Ok(())
        })
        .await
    }

    /// Newest audit rows first. `limit == 0` = all.
    pub async fn audit_log(&self, limit: u32) -> StoreResult<Vec<AuditEntry>> {
        self.read(move |conn| {
            let limit = if limit == 0 { -1 } else { i64::from(limit) };
            let mut stmt = conn.prepare_cached(
                "SELECT at, device_id, ip, action, target, success, detail FROM audit \
                 ORDER BY id DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map([limit], |r| {
                Ok(AuditEntry {
                    at: Millis(r.get(0)?),
                    device_id: r.get::<_, Option<String>>(1)?.map(DeviceId),
                    ip: r.get(2)?,
                    action: r.get(3)?,
                    target: r.get(4)?,
                    success: r.get(5)?,
                    detail: r.get(6)?,
                })
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .await
    }
}
