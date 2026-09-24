//! `FfiError`: the typed error that crosses the boundary (Swift `FfiError` enum, `throws`).

use osprey_domain::DomainError;

/// Error returned by every fallible FFI call. Each variant carries a human message
/// (already redacted by the engine); Swift switches on the case.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum FfiError {
    #[error("not found: {message}")]
    NotFound { message: String },
    #[error("validation failed: {message}")]
    Validation { message: String },
    #[error("conflict: {message}")]
    Conflict { message: String },
    #[error("permission denied: {message}")]
    PermissionDenied { message: String },
    #[error("storage error: {message}")]
    Storage { message: String },
    #[error("engine error: {message}")]
    Engine { message: String },
    #[error("unavailable: {message}")]
    Unavailable { message: String },
    #[error("internal error: {message}")]
    Internal { message: String },
}

pub type FfiResult<T> = Result<T, FfiError>;

impl FfiError {
    pub fn validation(message: impl std::fmt::Display) -> Self {
        Self::Validation {
            message: message.to_string(),
        }
    }
    pub fn internal(message: impl std::fmt::Display) -> Self {
        Self::Internal {
            message: message.to_string(),
        }
    }
    pub fn unavailable(message: impl std::fmt::Display) -> Self {
        Self::Unavailable {
            message: message.to_string(),
        }
    }
    pub fn not_found(message: impl std::fmt::Display) -> Self {
        Self::NotFound {
            message: message.to_string(),
        }
    }
}

impl From<DomainError> for FfiError {
    fn from(e: DomainError) -> Self {
        match e {
            DomainError::NotFound(message) => Self::NotFound { message },
            // The FFI keeps the spec's eight cases; an illegal state change is a conflict with
            // the task's current state.
            DomainError::InvalidTransition(message) | DomainError::Conflict(message) => {
                Self::Conflict { message }
            }
            DomainError::Validation(message) => Self::Validation { message },
            DomainError::PermissionDenied(message) => Self::PermissionDenied { message },
            DomainError::Storage(message) => Self::Storage { message },
            DomainError::Engine(message) => Self::Engine { message },
            DomainError::Unavailable(message) => Self::Unavailable { message },
            DomainError::Internal(message) => Self::Internal { message },
        }
    }
}

impl From<serde_json::Error> for FfiError {
    fn from(e: serde_json::Error) -> Self {
        Self::Validation {
            message: format!("invalid JSON: {e}"),
        }
    }
}

/// Render a panic payload for `FfiError::Internal`.
pub fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        format!("panic: {s}")
    } else if let Some(s) = payload.downcast_ref::<String>() {
        format!("panic: {s}")
    } else {
        "panic in the engine".to_owned()
    }
}
