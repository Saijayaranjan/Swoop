//! Millisecond Unix timestamps. A plain integer is the most portable representation across
//! SQLite, JSON and the FFI boundary.

use serde::{Deserialize, Serialize};

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Millis(pub i64);

impl Millis {
    pub fn now() -> Self {
        Self(chrono::Utc::now().timestamp_millis())
    }
    pub fn as_i64(self) -> i64 {
        self.0
    }
    pub fn to_datetime(self) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp_millis(self.0).unwrap_or_default()
    }
    pub fn saturating_add_ms(self, ms: i64) -> Self {
        Self(self.0.saturating_add(ms))
    }
    pub fn elapsed_since(self, earlier: Millis) -> i64 {
        self.0 - earlier.0
    }
}

impl From<i64> for Millis {
    fn from(v: i64) -> Self {
        Self(v)
    }
}
