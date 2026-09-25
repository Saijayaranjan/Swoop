//! Extractors that report failures in the API error shape instead of axum's plain-text defaults.

use crate::error::ApiError;
use axum::extract::{FromRequest, FromRequestParts, Path, Request};
use axum::http::request::Parts;
use axum::http::{header, StatusCode};
use bytes::Bytes;
use serde::de::DeserializeOwned;

/// Maximum length of an id path segment (ids are UUIDs; anything longer is junk).
const MAX_ID_LEN: usize = 200;

/// Read the body, enforcing `DefaultBodyLimit` and refusing non-JSON content types (a missing
/// `Content-Type` is tolerated so `POST` actions without a body keep working).
async fn read_json_body<S: Send + Sync>(req: Request, state: &S) -> Result<Bytes, ApiError> {
    if let Some(ct) = req.headers().get(header::CONTENT_TYPE) {
        let ct = ct.to_str().unwrap_or("").to_ascii_lowercase();
        if !ct.is_empty() && !ct.contains("json") {
            return Err(ApiError::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "validation",
                "expected Content-Type: application/json",
            ));
        }
    }
    Bytes::from_request(req, state).await.map_err(|e| {
        if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
            ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "validation",
                "request body too large",
            )
        } else {
            ApiError::validation(format!("cannot read request body: {e}"))
        }
    })
}

fn parse<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, ApiError> {
    serde_json::from_slice(bytes).map_err(|e| ApiError::validation(format!("invalid JSON: {e}")))
}

/// Required JSON body.
pub struct ApiJson<T>(pub T);

impl<T, S> FromRequest<S> for ApiJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;
    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = read_json_body(req, state).await?;
        if bytes.iter().all(|b| b.is_ascii_whitespace()) {
            return Err(ApiError::validation("request body required"));
        }
        Ok(Self(parse(&bytes)?))
    }
}

/// Optional JSON body: an empty body yields `T::default()`.
pub struct OptJson<T>(pub T);

impl<T, S> FromRequest<S> for OptJson<T>
where
    T: DeserializeOwned + Default,
    S: Send + Sync,
{
    type Rejection = ApiError;
    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = read_json_body(req, state).await?;
        if bytes.iter().all(|b| b.is_ascii_whitespace()) {
            return Ok(Self(T::default()));
        }
        Ok(Self(parse(&bytes)?))
    }
}

/// A single `{id}` path parameter.
pub struct Id(pub String);

impl<S: Send + Sync> FromRequestParts<S> for Id {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(id) = Path::<String>::from_request_parts(parts, state)
            .await
            .map_err(|e| ApiError::validation(format!("invalid path: {e}")))?;
        if id.is_empty() || id.len() > MAX_ID_LEN || id.chars().any(|c| c.is_control()) {
            return Err(ApiError::validation("invalid id"));
        }
        Ok(Self(id))
    }
}

/// Decoded query pairs (repeated keys preserved).
pub struct QueryPairs(pub Vec<(String, String)>);

impl<S: Send + Sync> FromRequestParts<S> for QueryPairs {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(query_pairs(parts.uri.query())))
    }
}

pub fn query_pairs(query: Option<&str>) -> Vec<(String, String)> {
    query
        .map(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect()
        })
        .unwrap_or_default()
}

impl QueryPairs {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .filter(|v| !v.is_empty())
    }

    /// All values for a key, also splitting comma-separated lists.
    pub fn all(&self, key: &str) -> Vec<String> {
        self.0
            .iter()
            .filter(|(k, _)| k == key)
            .flat_map(|(_, v)| v.split(','))
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect()
    }

    pub fn string(&self, key: &str) -> Option<String> {
        self.get(key).map(str::to_owned)
    }

    pub fn bool(&self, key: &str) -> Result<Option<bool>, ApiError> {
        match self.get(key) {
            None => Ok(None),
            Some("true" | "1" | "yes") => Ok(Some(true)),
            Some("false" | "0" | "no") => Ok(Some(false)),
            Some(v) => Err(ApiError::validation(format!(
                "{key}: expected a boolean, got {v:?}"
            ))),
        }
    }

    pub fn num<N: std::str::FromStr>(&self, key: &str) -> Result<Option<N>, ApiError> {
        match self.get(key) {
            None => Ok(None),
            Some(v) => v
                .parse::<N>()
                .map(Some)
                .map_err(|_| ApiError::validation(format!("{key}: expected a number, got {v:?}"))),
        }
    }

    /// Parse a snake_case enum value through serde.
    pub fn enum_value<T: DeserializeOwned>(key: &str, v: &str) -> Result<T, ApiError> {
        serde_json::from_value(serde_json::Value::String(v.to_owned()))
            .map_err(|_| ApiError::validation(format!("{key}: unknown value {v:?}")))
    }

    pub fn opt_enum<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, ApiError> {
        self.get(key).map(|v| Self::enum_value(key, v)).transpose()
    }
}
