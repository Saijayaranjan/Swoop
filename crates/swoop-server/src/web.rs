//! The embedded remote web UI (`web/remote/dist`), served with an SPA fallback to `index.html`.
//! If the bundle was not built before compiling, a short page explains how to build it.

use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../web/remote/dist"]
#[allow_missing = true]
struct Assets;

const MISSING_UI: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>Swoop</title>
<meta name="viewport" content="width=device-width, initial-scale=1"></head>
<body style="font-family: system-ui, sans-serif; max-width: 40rem; margin: 3rem auto; padding: 0 1rem">
<h1>Swoop web UI not built</h1>
<p>This server was compiled without the remote web UI bundle. The REST API at
<code>/api/v1</code> works normally.</p>
<p>To include the UI, build it and then rebuild Swoop:</p>
<pre>cd web/remote
npm install
npm run build
cargo build -p swoop-cli</pre>
</body></html>"#;

pub(crate) async fn fallback(State(st): State<AppState>, req: Request) -> Response {
    let path = req.uri().path();
    if path.starts_with("/api/") || path == "/api" {
        return ApiError::not_found("no such API route").into_response();
    }
    if req.method() != Method::GET && req.method() != Method::HEAD {
        return ApiError::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "validation",
            "method not allowed",
        )
        .into_response();
    }
    if !st.cfg.serve_web_ui {
        return ApiError::not_found("the web UI is disabled").into_response();
    }
    let rel = path.trim_start_matches('/');
    if !rel.is_empty() && !rel.split('/').any(|seg| seg == ".." || seg.is_empty()) {
        if let Some(file) = Assets::get(rel) {
            let mime = mime_guess::from_path(rel).first_or_octet_stream();
            let mut resp = file.data.into_owned().into_response();
            let h = resp.headers_mut();
            if let Ok(v) = HeaderValue::from_str(mime.as_ref()) {
                h.insert(header::CONTENT_TYPE, v);
            }
            // Bundles have content-hashed names; everything else revalidates.
            let immutable = rel.starts_with("main-");
            h.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static(if immutable {
                    "public, max-age=31536000, immutable"
                } else {
                    "no-cache"
                }),
            );
            return resp;
        }
        // A missing asset with an extension is a real 404, not a client-side route.
        let last = rel.rsplit('/').next().unwrap_or("");
        if last.contains('.') {
            return (StatusCode::NOT_FOUND, "not found").into_response();
        }
    }
    match Assets::get("index.html") {
        Some(index) => {
            let mut resp = Html(index.data.into_owned()).into_response();
            resp.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
            resp
        }
        None => {
            let mut resp = Html(MISSING_UI).into_response();
            // The explanatory page uses one inline style attribute.
            resp.headers_mut().insert(
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static(
                    "default-src 'none'; style-src 'unsafe-inline'; frame-ancestors 'none'",
                ),
            );
            resp
        }
    }
}
