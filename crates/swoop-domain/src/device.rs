//! Paired remote devices and their permission scopes.

use crate::{DeviceId, Millis};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// List tasks, queues, stats.
    Read,
    /// Add tasks.
    Add,
    /// Pause/resume/retry/remove, change limits.
    Control,
    /// Manage devices, settings, automation.
    Admin,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Read => "read",
            Scope::Add => "add",
            Scope::Control => "control",
            Scope::Admin => "admin",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "read" => Some(Scope::Read),
            "add" => Some(Scope::Add),
            "control" => Some(Scope::Control),
            "admin" => Some(Scope::Admin),
            _ => None,
        }
    }
    /// Admin implies everything; Control implies Read and Add. `Add` alone is deliberately
    /// narrow (a share-sheet device may add URLs without listing what else was downloaded).
    pub fn implies(self, other: Scope) -> bool {
        match self {
            Scope::Admin => true,
            Scope::Control => matches!(other, Scope::Read | Scope::Add | Scope::Control),
            Scope::Add => other == Scope::Add,
            Scope::Read => other == Scope::Read,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub id: DeviceId,
    pub name: String,
    /// `phone`, `tablet`, `browser`, `computer`, `cli`, `extension`.
    pub kind: String,
    pub scopes: Vec<Scope>,
    pub created_at: Millis,
    #[serde(default)]
    pub last_seen_at: Option<Millis>,
    #[serde(default)]
    pub last_ip: Option<String>,
    #[serde(default)]
    pub expires_at: Option<Millis>,
    #[serde(default)]
    pub revoked: bool,
}

impl Device {
    pub fn has_scope(&self, s: Scope) -> bool {
        !self.revoked && self.scopes.iter().any(|d| d.implies(s))
    }
}

/// Audit log row for the remote API.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub at: Millis,
    pub device_id: Option<DeviceId>,
    pub ip: String,
    pub action: String,
    pub target: Option<String>,
    pub success: bool,
    pub detail: Option<String>,
}
