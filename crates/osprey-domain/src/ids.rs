//! Strongly typed identifiers. All ids are UUIDv4 strings so they are trivially portable across
//! SQLite, JSON, the FFI boundary and URLs.

use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            /// Generate a fresh random id.
            pub fn new() -> Self {
                Self(uuid::Uuid::new_v4().to_string())
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }
        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_owned())
            }
        }
        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

define_id!(
    #[doc = "Identifies a download task."]
    TaskId
);
define_id!(
    #[doc = "Identifies a queue."]
    QueueId
);
define_id!(
    #[doc = "Identifies a category."]
    CategoryId
);
define_id!(
    #[doc = "Identifies an organisation rule."]
    RuleId
);
define_id!(
    #[doc = "Identifies a schedule."]
    ScheduleId
);
define_id!(
    #[doc = "Identifies an automation rule."]
    AutomationId
);
define_id!(
    #[doc = "Identifies a paired remote device."]
    DeviceId
);
define_id!(
    #[doc = "Identifies a stored credential (the secret itself lives in the OS keychain)."]
    CredentialId
);
define_id!(
    #[doc = "Identifies a proxy profile."]
    ProxyId
);
define_id!(
    #[doc = "Identifies a saved download recipe."]
    RecipeId
);
define_id!(
    #[doc = "Identifies a plugin."]
    PluginId
);

impl QueueId {
    /// The built-in default queue. Its id is stable so it survives export/import.
    pub fn default_queue() -> Self {
        Self("queue-default".to_owned())
    }
}
