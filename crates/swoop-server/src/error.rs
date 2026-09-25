//! The API error shape: `{"error": {"type": "...", "message": "..."}}` with a matching status.

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use swoop_domain::DomainError;
use swoop_runtime::redact::redact;

#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: StatusCode,
    pub kind: &'static str,
    pub message: String,
    /// Seconds, for 429 responses.
    pub retry_after: Option<u64>,
}

impl ApiError {
    pub fn new(status: StatusCode, kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            kind,
            message: message.into(),
            retry_after: None,
        }
    }
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "permission_denied", message)
    }
    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "validation", message)
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", message)
    }
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", message)
    }
    pub fn rate_limited(retry_after: u64) -> Self {
        Self {
            retry_after: Some(retry_after.max(1)),
            ..Self::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "too many requests",
            )
        }
    }
}

impl From<DomainError> for ApiError {
    fn from(e: DomainError) -> Self {
        let (status, kind, msg) = match e {
            DomainError::NotFound(m) => (StatusCode::NOT_FOUND, "not_found", m),
            DomainError::InvalidTransition(m) => (StatusCode::CONFLICT, "conflict", m),
            DomainError::Validation(m) => (StatusCode::BAD_REQUEST, "validation", m),
            DomainError::Conflict(m) => (StatusCode::CONFLICT, "conflict", m),
            DomainError::PermissionDenied(m) => (StatusCode::FORBIDDEN, "permission_denied", m),
            DomainError::Storage(m) => (StatusCode::INTERNAL_SERVER_ERROR, "storage", m),
            DomainError::Engine(m) => (StatusCode::INTERNAL_SERVER_ERROR, "engine", m),
            DomainError::Unavailable(m) => (StatusCode::SERVICE_UNAVAILABLE, "unavailable", m),
            DomainError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, "internal", m),
        };
        Self::new(status, kind, msg)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({
            "error": { "type": self.kind, "message": redact(&self.message) }
        });
        let mut resp = (self.status, axum::Json(body)).into_response();
        if let Some(s) = self.retry_after {
            if let Ok(v) = HeaderValue::from_str(&s.to_string()) {
                resp.headers_mut().insert(header::RETRY_AFTER, v);
            }
        }
        if self.status == StatusCode::UNAUTHORIZED {
            resp.headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        resp
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
