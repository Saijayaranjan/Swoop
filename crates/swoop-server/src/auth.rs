//! Authentication, authorization, origin checks, rate limiting and the remote audit trail.
//!
//! [`policy`] is the single table mapping every route to the scope it needs. Anything not in the
//! table requires `admin` (fail closed).

use crate::error::ApiError;
use crate::extract::query_pairs;
use crate::state::{AppState, Caller, ClientIp};
use crate::ListenerKind;
use axum::extract::{ConnectInfo, MatchedPath, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::net::SocketAddr;
use swoop_domain::device::{AuditEntry, Scope};
use swoop_domain::{DomainError, Millis};

/// What a route requires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Need {
    /// No authentication (`/healthz`, `POST /api/v1/pair`).
    Public,
    Scope(Scope),
    /// Admin, or the device acting on its own record (`DELETE /devices/{id}` = "forget me").
    SelfOrAdmin,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Policy {
    pub need: Need,
    /// Only the local token on a local listener may call this route.
    pub trusted_only: bool,
    /// `?token=` is accepted in addition to the `Authorization` header.
    pub query_token: bool,
}

impl Policy {
    const fn scope(s: Scope) -> Self {
        Self {
            need: Need::Scope(s),
            trusted_only: false,
            query_token: false,
        }
    }
}

/// Scope table from `docs/api/rest.md`. R = read, A = add, C = control, X = admin.
pub(crate) fn policy(method: &Method, path: &str) -> Policy {
    use Scope::{Add as A, Admin as X, Control as C, Read as R};
    let p = path.strip_prefix("/api/v1").unwrap_or(path);
    let get = method == Method::GET || method == Method::HEAD;
    let s = |scope| Policy::scope(scope);

    match p {
        "/healthz" | "/pair" => {
            return Policy {
                need: Need::Public,
                trusted_only: false,
                query_token: false,
            }
        }
        "/events" => {
            return Policy {
                query_token: true,
                ..s(R)
            }
        }
        "/tasks/{id}/file" => {
            return Policy {
                query_token: true,
                ..s(R)
            }
        }
        "/archives/list" => {
            return Policy {
                trusted_only: true,
                ..s(R)
            }
        }
        "/archives/extract" => {
            return Policy {
                trusted_only: true,
                ..s(C)
            }
        }
        _ => {}
    }

    // Engine
    match p {
        "/info" | "/stats" | "/dashboard" | "/disk" => return s(R),
        "/settings" | "/environment" | "/logs" => return s(X),
        "/traffic-mode" | "/limits" | "/optimize" => return s(C),
        _ => {}
    }

    // Tasks
    if p == "/tasks/probe" || p == "/tasks/batch" || p == "/media/detect" {
        return s(A);
    }
    if p == "/tasks" {
        return if get { s(R) } else { s(A) };
    }
    if p == "/tasks/rows" {
        return s(R);
    }
    if p.starts_with("/tasks/") {
        return if get { s(R) } else { s(C) };
    }
    if p == "/trackers/refresh" {
        return s(X);
    }

    // Queues: R for reads, C for pause/resume, X for other mutations.
    if p == "/queues/{id}/pause" || p == "/queues/{id}/resume" {
        return s(C);
    }
    // Rules dry-run is a read.
    if p == "/rules/test" {
        return s(R);
    }
    // Applying a recipe adds a task.
    if p == "/recipes/{id}/apply" {
        return s(A);
    }
    for prefix in [
        "/queues",
        "/categories",
        "/rules",
        "/schedules",
        "/automations",
        "/recipes",
    ] {
        if p == prefix || p.starts_with(&format!("{prefix}/")) {
            return if get { s(R) } else { s(X) };
        }
    }

    // History
    match p {
        "/history" => return if get { s(R) } else { s(X) },
        "/history/count" => return s(R),
        "/history/delete" => return s(C),
        _ => {}
    }

    // Devices
    if p == "/devices/{id}" && method == Method::DELETE {
        return Policy {
            need: Need::SelfOrAdmin,
            trusted_only: false,
            query_token: false,
        };
    }
    if p == "/devices" || p.starts_with("/devices/") || p == "/audit" {
        return s(X);
    }

    // Grabber
    if p == "/grabber" {
        return if get { s(R) } else { s(A) };
    }
    if p == "/grabber/{id}" {
        return if get { s(R) } else { s(C) };
    }
    if p == "/grabber/{id}/add" {
        return s(A);
    }

    // import/export, updates, plugins and anything unknown: admin.
    s(X)
}

fn client_ip(req: &Request, kind: ListenerKind) -> String {
    if let Some(ConnectInfo(addr)) = req.extensions().get::<ConnectInfo<SocketAddr>>() {
        return addr.ip().to_string();
    }
    match kind {
        // Unix socket connections carry no address.
        ListenerKind::Local => "unix".to_owned(),
        ListenerKind::Remote => "unknown".to_owned(),
    }
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?.trim();
    let (scheme, token) = v.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    (!token.is_empty()).then(|| token.to_owned())
}

pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Route guard (installed with `route_layer`, so `MatchedPath` is available): resolves the
/// caller, enforces the policy table and the rate limit, and audits remote requests.
pub(crate) async fn guard(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    let kind = state.cfg.kind;
    let ip = client_ip(&req, kind);
    req.extensions_mut().insert(ClientIp(ip.clone()));
    let method = req.method().clone();
    let matched = req
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_owned())
        .unwrap_or_else(|| req.uri().path().to_owned());
    // Never the query: it may carry a token.
    let target = req.uri().path().to_owned();
    let pol = policy(&method, &matched);

    let outcome = authorize(&state, req.headers(), req.uri(), &pol, &ip).await;
    let (resp, device_id) = match outcome {
        Ok(caller) => {
            let device_id = caller.as_ref().map(|c| c.device.id.clone());
            if let Some(c) = caller {
                req.extensions_mut().insert(c);
            }
            (next.run(req).await, device_id)
        }
        Err((e, device_id)) => (e.into_response(), device_id),
    };

    if kind == ListenerKind::Remote && matched != "/healthz" {
        let status = resp.status();
        let entry = AuditEntry {
            at: Millis::now(),
            device_id,
            ip,
            action: format!("{method} {matched}"),
            target: Some(target),
            success: status.is_success()
                || status.is_redirection()
                || status == StatusCode::SWITCHING_PROTOCOLS,
            detail: Some(status.as_u16().to_string()),
        };
        let engine = state.engine.clone();
        tokio::spawn(async move {
            if let Err(e) = engine.record_audit(entry).await {
                tracing::warn!(error = %e, "failed to write audit entry");
            }
        });
    }
    resp
}

type AuthError = (ApiError, Option<swoop_domain::DeviceId>);

async fn authorize(
    state: &AppState,
    headers: &HeaderMap,
    uri: &axum::http::Uri,
    pol: &Policy,
    ip: &str,
) -> Result<Option<Caller>, AuthError> {
    let kind = state.cfg.kind;
    if pol.need == Need::Public {
        // Unauthenticated routes are rate limited per client address on the remote listener.
        state
            .limiter
            .check(&format!("ip:{ip}"))
            .map_err(|wait| (ApiError::rate_limited(wait), None))?;
        return Ok(None);
    }

    let token = bearer(headers).or_else(|| {
        if pol.query_token {
            query_pairs(uri.query())
                .into_iter()
                .rev()
                .find(|(k, _)| k == "token")
                .map(|(_, v)| v)
                .filter(|v| !v.is_empty())
        } else {
            None
        }
    });
    let Some(token) = token else {
        return Err((ApiError::unauthorized("missing bearer token"), None));
    };
    if token.len() > 512 {
        return Err((ApiError::unauthorized("invalid token"), None));
    }

    let is_local_token = constant_time_eq(token.as_bytes(), state.engine.local_token().as_bytes());
    if is_local_token && kind == ListenerKind::Remote {
        // The local token never works over the network, even if it leaked.
        return Err((ApiError::unauthorized("invalid token"), None));
    }

    let device = match state.engine.authenticate(&token, ip).await {
        Ok(d) => d,
        Err(DomainError::PermissionDenied(_)) => {
            // Lockout after repeated failures from this address.
            return Err((ApiError::rate_limited(60), None));
        }
        Err(DomainError::NotFound(_)) => {
            return Err((ApiError::unauthorized("invalid or expired token"), None))
        }
        Err(e) => return Err((ApiError::from(e), None)),
    };
    let caller = Caller {
        trusted: is_local_token && kind == ListenerKind::Local,
        device,
    };
    let device_id = Some(caller.device.id.clone());

    if !caller.trusted {
        state
            .limiter
            .check(caller.device.id.as_str())
            .map_err(|wait| (ApiError::rate_limited(wait), device_id.clone()))?;
    }

    if pol.trusted_only && !caller.trusted {
        return Err((
            ApiError::forbidden("this operation is only available to the local user"),
            device_id,
        ));
    }

    let allowed = match pol.need {
        Need::Public => true,
        Need::Scope(s) => caller.has(s),
        Need::SelfOrAdmin => {
            caller.has(Scope::Admin)
                || uri
                    .path()
                    .rsplit('/')
                    .next()
                    .is_some_and(|id| id == caller.device.id.as_str())
        }
    };
    if !allowed {
        let needed = match pol.need {
            Need::Scope(s) => s.as_str(),
            _ => "admin",
        };
        return Err((
            ApiError::forbidden(format!("this token lacks the '{needed}' scope")),
            device_id,
        ));
    }
    Ok(Some(caller))
}

// ---------------------------------------------------------------------------------------------
// Origin checks + CORS
// ---------------------------------------------------------------------------------------------

const EXTENSION_SCHEMES: [&str; 3] = [
    "chrome-extension://",
    "moz-extension://",
    "safari-web-extension://",
];

fn normalise_origin(o: &str) -> String {
    o.trim().trim_end_matches('/').to_ascii_lowercase()
}

/// Is `origin` acceptable for a request that arrived with `Host: host`?
pub(crate) fn origin_allowed(
    origin: &str,
    host: Option<&str>,
    configured: &[String],
    from_settings: &[String],
) -> bool {
    let o = normalise_origin(origin);
    if o.is_empty() || o == "null" {
        return false;
    }
    if EXTENSION_SCHEMES.iter().any(|s| o.starts_with(s)) {
        return true;
    }
    if let Some(h) = host {
        let h = h.trim().to_ascii_lowercase();
        if !h.is_empty() && (o == format!("http://{h}") || o == format!("https://{h}")) {
            return true;
        }
    }
    configured
        .iter()
        .chain(from_settings)
        .any(|a| normalise_origin(a) == o)
}

/// Whole-app middleware: rejects requests whose `Origin` is not allowed, answers CORS
/// preflights for allowed cross-origin callers and decorates their responses.
pub(crate) async fn origin_check(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let Some(origin) = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
    else {
        return next.run(req).await;
    };
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .or_else(|| req.uri().authority().map(|a| a.to_string()));
    let settings = state.engine.settings();
    if !origin_allowed(
        &origin,
        host.as_deref(),
        &state.cfg.allowed_origins,
        &settings.remote.allowed_origins,
    ) {
        return ApiError::forbidden("origin not allowed").into_response();
    }
    let same_origin = host.as_deref().is_some_and(|h| {
        let o = normalise_origin(&origin);
        let h = h.to_ascii_lowercase();
        o == format!("http://{h}") || o == format!("https://{h}")
    });

    let preflight = req.method() == Method::OPTIONS
        && req
            .headers()
            .contains_key(header::ACCESS_CONTROL_REQUEST_METHOD);
    let mut resp = if preflight {
        StatusCode::NO_CONTENT.into_response()
    } else {
        next.run(req).await
    };
    if !same_origin || preflight {
        if let Ok(v) = HeaderValue::from_str(&origin) {
            let h = resp.headers_mut();
            h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, v);
            h.append(header::VARY, HeaderValue::from_static("Origin"));
            if preflight {
                h.insert(
                    header::ACCESS_CONTROL_ALLOW_METHODS,
                    HeaderValue::from_static("GET, POST, PUT, PATCH, DELETE, OPTIONS"),
                );
                h.insert(
                    header::ACCESS_CONTROL_ALLOW_HEADERS,
                    HeaderValue::from_static("authorization, content-type"),
                );
                h.insert(
                    header::ACCESS_CONTROL_MAX_AGE,
                    HeaderValue::from_static("600"),
                );
            } else {
                h.insert(
                    header::ACCESS_CONTROL_EXPOSE_HEADERS,
                    HeaderValue::from_static("retry-after, content-disposition"),
                );
            }
        }
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_table() {
        let g = Method::GET;
        let p = Method::POST;
        assert_eq!(policy(&g, "/api/v1/tasks").need, Need::Scope(Scope::Read));
        assert_eq!(policy(&p, "/api/v1/tasks").need, Need::Scope(Scope::Add));
        assert_eq!(
            policy(&p, "/api/v1/tasks/{id}/pause").need,
            Need::Scope(Scope::Control)
        );
        assert_eq!(
            policy(&g, "/api/v1/settings").need,
            Need::Scope(Scope::Admin)
        );
        assert_eq!(policy(&p, "/api/v1/queues").need, Need::Scope(Scope::Admin));
        assert_eq!(
            policy(&p, "/api/v1/queues/{id}/pause").need,
            Need::Scope(Scope::Control)
        );
        assert_eq!(policy(&p, "/api/v1/pair").need, Need::Public);
        assert!(policy(&p, "/api/v1/archives/extract").trusted_only);
        assert!(policy(&g, "/api/v1/events").query_token);
        assert!(!policy(&g, "/api/v1/tasks").query_token);
        assert_eq!(
            policy(&g, "/api/v1/whatever").need,
            Need::Scope(Scope::Admin)
        );
        assert_eq!(
            policy(&Method::DELETE, "/api/v1/devices/{id}").need,
            Need::SelfOrAdmin
        );
    }

    #[test]
    fn origins() {
        let none: Vec<String> = vec![];
        assert!(origin_allowed("chrome-extension://abc", None, &none, &none));
        assert!(origin_allowed("moz-extension://x", None, &none, &none));
        assert!(origin_allowed(
            "https://host:41780",
            Some("host:41780"),
            &none,
            &none
        ));
        assert!(!origin_allowed(
            "https://evil.example",
            Some("host:41780"),
            &none,
            &none
        ));
        assert!(!origin_allowed("null", Some("host"), &none, &none));
        assert!(origin_allowed(
            "https://nas.local:8443",
            Some("host"),
            &["https://NAS.local:8443/".into()],
            &none
        ));
    }
}
