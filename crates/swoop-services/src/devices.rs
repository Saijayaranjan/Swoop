//! Remote devices: pairing codes (single-use, 120 s TTL), bearer tokens (32 random bytes,
//! base64url; SHA-256 hex stored), per-IP lockout and the audit log.

use crate::api::PairingInfo;
use crate::engine::Engine;
use parking_lot::Mutex;
use rand::RngCore;
use std::collections::HashMap;
use swoop_domain::device::{AuditEntry, Device, Scope};
use swoop_domain::{DeviceId, DomainError, DomainResult, Event, Millis, Notification};

/// Pairing code lifetime.
pub const PAIRING_TTL_MS: i64 = 120_000;
/// Alphabet without ambiguous glyphs (0/O, 1/I/L).
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
/// Id of the synthetic device returned for the local API token.
pub const LOCAL_DEVICE_ID: &str = "local";

#[derive(Clone, Debug)]
struct Pairing {
    code: String,
    scopes: Vec<Scope>,
    expires_at: Millis,
}

#[derive(Clone, Debug, Default)]
struct Lockout {
    failures: u32,
    locked_until: Option<Millis>,
}

/// Pairing and lockout state.
#[derive(Default)]
pub struct DeviceManager {
    pairing: Mutex<Option<Pairing>>,
    lockouts: Mutex<HashMap<String, Lockout>>,
}

/// Generate a pairing code (`XXXX-XXXX`).
pub fn generate_code() -> String {
    let mut rng = rand::thread_rng();
    let mut s = String::with_capacity(9);
    for i in 0..8 {
        if i == 4 {
            s.push('-');
        }
        let idx = (rng.next_u32() as usize) % CODE_ALPHABET.len();
        s.push(CODE_ALPHABET[idx] as char);
    }
    s
}

/// Normalise user input: upper-case, strip separators/spaces.
fn normalise_code(code: &str) -> String {
    code.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Fresh bearer token: 32 random bytes, base64url without padding.
pub fn generate_token() -> String {
    use base64::Engine as _;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// SHA-256 hex of a token (what the store keeps).
pub fn token_hash(token: &str) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(token.as_bytes()))
}

/// The synthetic admin device for the local API token.
pub fn local_device() -> Device {
    Device {
        id: DeviceId(LOCAL_DEVICE_ID.to_owned()),
        name: "This computer".into(),
        kind: "cli".into(),
        scopes: vec![Scope::Admin],
        created_at: Millis(0),
        last_seen_at: Some(Millis::now()),
        last_ip: Some("127.0.0.1".into()),
        expires_at: None,
        revoked: false,
    }
}

impl Engine {
    pub(crate) async fn start_pairing_inner(
        &self,
        scopes: Vec<Scope>,
    ) -> DomainResult<PairingInfo> {
        let scopes = if scopes.is_empty() {
            vec![Scope::Read, Scope::Add, Scope::Control]
        } else {
            scopes
        };
        let code = generate_code();
        let expires_at = Millis::now().saturating_add_ms(PAIRING_TTL_MS);
        *self.devices.pairing.lock() = Some(Pairing {
            code: normalise_code(&code),
            scopes,
            expires_at,
        });
        let settings = self.settings();
        let host = sysinfo::System::host_name().unwrap_or_else(|| "localhost".into());
        let url = format!(
            "{}://{}:{}/pair",
            if settings.remote.tls { "https" } else { "http" },
            host,
            settings.remote.port
        );
        self.bus.publish(Event::PairingStarted {
            code: code.clone(),
            expires_at,
            url: url.clone(),
        });
        Ok(PairingInfo {
            code,
            expires_at,
            url,
            tls_fingerprint: None,
        })
    }

    pub(crate) async fn cancel_pairing_inner(&self) -> DomainResult<()> {
        *self.devices.pairing.lock() = None;
        Ok(())
    }

    fn check_lockout(&self, ip: &str) -> DomainResult<()> {
        let now = Millis::now();
        let mut l = self.devices.lockouts.lock();
        if let Some(entry) = l.get_mut(ip) {
            if let Some(until) = entry.locked_until {
                if until.0 > now.0 {
                    return Err(DomainError::PermissionDenied(format!(
                        "too many failed attempts; locked for {} s",
                        (until.0 - now.0) / 1000
                    )));
                }
                entry.locked_until = None;
                entry.failures = 0;
            }
        }
        Ok(())
    }

    fn record_failure(&self, ip: &str) {
        let settings = self.settings();
        let mut l = self.devices.lockouts.lock();
        let entry = l.entry(ip.to_owned()).or_default();
        entry.failures += 1;
        if entry.failures >= settings.remote.max_failed_attempts.max(1) {
            entry.locked_until = Some(
                Millis::now()
                    .saturating_add_ms(i64::from(settings.remote.lockout_minutes) * 60_000),
            );
        }
    }

    fn clear_failures(&self, ip: &str) {
        self.devices.lockouts.lock().remove(ip);
    }

    pub(crate) async fn complete_pairing_inner(
        &self,
        code: String,
        device_name: String,
        device_kind: String,
        ip: String,
    ) -> DomainResult<(Device, String)> {
        self.check_lockout(&ip)?;
        let now = Millis::now();
        let pairing = self.devices.pairing.lock().clone();
        let attempt = normalise_code(&code);
        let valid = pairing
            .as_ref()
            .map(|p| {
                p.expires_at.0 > now.0 && constant_time_eq(p.code.as_bytes(), attempt.as_bytes())
            })
            .unwrap_or(false);
        if !valid {
            self.record_failure(&ip);
            let _ = self
                .store
                .append_audit(&AuditEntry {
                    at: now,
                    device_id: None,
                    ip: ip.clone(),
                    action: "pairing.failed".into(),
                    target: None,
                    success: false,
                    detail: Some(
                        if pairing.is_none() {
                            "no pairing in progress"
                        } else {
                            "wrong or expired code"
                        }
                        .into(),
                    ),
                })
                .await;
            return Err(DomainError::PermissionDenied("invalid pairing code".into()));
        }
        // single use
        let pairing = self
            .devices
            .pairing
            .lock()
            .take()
            .unwrap_or_else(|| Pairing {
                code: String::new(),
                scopes: Vec::new(),
                expires_at: now,
            });
        self.clear_failures(&ip);
        let settings = self.settings();
        let token = generate_token();
        let name = swoop_runtime::safety::sanitize_filename(device_name.trim());
        let kind = match device_kind.trim() {
            k @ ("phone" | "tablet" | "browser" | "computer" | "cli" | "extension") => k.to_owned(),
            _ => "computer".to_owned(),
        };
        let device = Device {
            id: DeviceId::new(),
            name,
            kind,
            scopes: pairing.scopes,
            created_at: now,
            last_seen_at: Some(now),
            last_ip: Some(ip.clone()),
            expires_at: Some(
                now.saturating_add_ms(i64::from(settings.remote.session_ttl_hours) * 3_600_000),
            ),
            revoked: false,
        };
        self.store
            .upsert_device(&device, Some(token_hash(&token)))
            .await?;
        self.store
            .append_audit(&AuditEntry {
                at: now,
                device_id: Some(device.id.clone()),
                ip,
                action: "pairing.completed".into(),
                target: Some(device.name.clone()),
                success: true,
                detail: None,
            })
            .await?;
        self.bus.publish(Event::PairingCompleted {
            device: device.clone(),
        });
        self.bus.publish(Event::DeviceUpdated(device.clone()));
        self.notify(Notification::DevicePaired {
            device_id: device.id.clone(),
            name: device.name.clone(),
        });
        Ok((device, token))
    }

    pub(crate) async fn authenticate_inner(&self, token: &str, ip: &str) -> DomainResult<Device> {
        if constant_time_eq(token.as_bytes(), self.local_token.as_bytes()) {
            return Ok(local_device());
        }
        self.check_lockout(ip)?;
        let now = Millis::now();
        let found = self
            .store
            .find_device_by_token_hash(&token_hash(token))
            .await?;
        let device = match found {
            Some(d) if !d.revoked && d.expires_at.map(|e| e.0 > now.0).unwrap_or(true) => d,
            _ => {
                self.record_failure(ip);
                return Err(DomainError::NotFound("unknown or revoked token".into()));
            }
        };
        self.clear_failures(ip);
        self.store.touch_device(&device.id, ip, now).await?;
        Ok(Device {
            last_seen_at: Some(now),
            last_ip: Some(ip.to_owned()),
            ..device
        })
    }

    pub(crate) async fn revoke_device_inner(&self, id: DeviceId) -> DomainResult<()> {
        let devices = self.store.list_devices().await?;
        let mut device = devices
            .into_iter()
            .find(|d| d.id == id)
            .ok_or_else(|| DomainError::not_found(format!("device {id}")))?;
        device.revoked = true;
        self.store.upsert_device(&device, None).await?;
        self.store
            .append_audit(&AuditEntry {
                at: Millis::now(),
                device_id: Some(id.clone()),
                ip: "local".into(),
                action: "device.revoked".into(),
                target: Some(device.name.clone()),
                success: true,
                detail: None,
            })
            .await?;
        self.bus.publish(Event::DeviceRemoved { device_id: id });
        Ok(())
    }

    pub(crate) async fn rename_device_inner(
        &self,
        id: DeviceId,
        name: String,
    ) -> DomainResult<Device> {
        let devices = self.store.list_devices().await?;
        let mut device = devices
            .into_iter()
            .find(|d| d.id == id)
            .ok_or_else(|| DomainError::not_found(format!("device {id}")))?;
        let name = name.trim();
        if name.is_empty() {
            return Err(DomainError::validation("device name must not be empty"));
        }
        device.name = swoop_runtime::safety::sanitize_filename(name);
        self.store.upsert_device(&device, None).await?;
        self.bus.publish(Event::DeviceUpdated(device.clone()));
        Ok(device)
    }

    pub(crate) async fn list_devices_inner(&self) -> DomainResult<Vec<Device>> {
        Ok(self.store.list_devices().await?)
    }

    pub(crate) async fn audit_log_inner(&self, limit: u32) -> DomainResult<Vec<AuditEntry>> {
        Ok(self.store.audit_log(limit).await?)
    }

    pub(crate) async fn record_audit_inner(&self, mut entry: AuditEntry) -> DomainResult<()> {
        entry.detail = entry.detail.map(|d| swoop_runtime::redact::redact(&d));
        entry.target = entry.target.map(|d| swoop_runtime::redact::redact(&d));
        Ok(self.store.append_audit(&entry).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_tokens() {
        let c = generate_code();
        assert_eq!(c.len(), 9);
        assert_eq!(normalise_code("abcd-efgh"), "ABCDEFGH");
        let t = generate_token();
        assert!(t.len() >= 42);
        assert_eq!(token_hash("x").len(), 64);
        assert!(constant_time_eq(b"ab", b"ab"));
        assert!(!constant_time_eq(b"ab", b"ac"));
    }
}
