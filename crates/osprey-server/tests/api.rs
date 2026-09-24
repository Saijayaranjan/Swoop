//! Essential end-to-end checks against a real engine: authentication, one CRUD flow, pairing +
//! scopes, origin checks and the WebSocket hello.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use futures::{SinkExt, StreamExt};
use osprey_server::{router, ListenerKind, ServerOptions};
use osprey_services::{EngineConfig, SharedEngine};
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

struct Env {
    engine: SharedEngine,
    _data: TempDir,
    _downloads: TempDir,
}

async fn engine() -> Env {
    std::env::set_var("OSPREY_CREDENTIALS_FILE_STORE", "1");
    let data = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let engine = osprey_services::bootstrap::start(EngineConfig {
        data_dir: Some(data.path().to_path_buf()),
        download_dir: Some(downloads.path().to_path_buf()),
        headless: true,
        skip_instance_lock: true,
        ..Default::default()
    })
    .await
    .expect("engine starts");
    Env {
        engine,
        _data: data,
        _downloads: downloads,
    }
}

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let req = match body {
        Some(b) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(b.to_string()))
            .unwrap(),
        None => req.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, v)
}

#[tokio::test(flavor = "multi_thread")]
async fn auth_and_task_crud() {
    let env = engine().await;
    let local = router(env.engine.clone(), ListenerKind::Local);
    let remote = router(env.engine.clone(), ListenerKind::Remote);
    let token = env.engine.local_token();

    // Unauthenticated / wrong token.
    let (s, v) = call(&local, "GET", "/api/v1/tasks", None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(v["error"]["type"], "unauthorized");
    let (s, _) = call(&local, "GET", "/api/v1/tasks", Some("nope"), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    // The local token never works on the remote listener.
    let (s, _) = call(&remote, "GET", "/api/v1/tasks", Some(&token), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    // Health is public.
    let (s, v) = call(&remote, "GET", "/healthz", None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["status"], "ok");

    // List, add, get, delete.
    let (s, v) = call(&local, "GET", "/api/v1/tasks", Some(&token), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["tasks"].is_array());

    let (s, v) = call(
        &local,
        "POST",
        "/api/v1/tasks",
        Some(&token),
        Some(json!({ "url": "http://127.0.0.1:9/file.bin", "start": false })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["task"]["id"].as_str().expect("task id").to_owned();

    let (s, v) = call(&local, "GET", &format!("/api/v1/tasks/{id}"), Some(&token), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["id"], id.as_str());

    let (s, v) = call(&local, "GET", "/api/v1/tasks/rows?state=pending&state=queued", Some(&token), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // Not complete → no file.
    let (s, _) = call(&local, "GET", &format!("/api/v1/tasks/{id}/file"), Some(&token), None).await;
    assert_eq!(s, StatusCode::CONFLICT);

    let (s, _) = call(&local, "DELETE", &format!("/api/v1/tasks/{id}"), Some(&token), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, v) = call(&local, "GET", &format!("/api/v1/tasks/{id}"), Some(&token), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(v["error"]["type"], "not_found");

    // Bad JSON is a validation error in the API shape.
    let (s, v) = call(&local, "POST", "/api/v1/limits", Some(&token), Some(json!({"download": "x"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(v["error"]["type"], "validation");

    // Foreign browser origins are refused; extension origins are allowed.
    let req = Request::get("/api/v1/info")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::ORIGIN, "https://evil.example")
        .body(Body::empty())
        .unwrap();
    assert_eq!(local.clone().oneshot(req).await.unwrap().status(), StatusCode::FORBIDDEN);
    let req = Request::get("/api/v1/info")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::ORIGIN, "chrome-extension://abcdef")
        .body(Body::empty())
        .unwrap();
    let resp = local.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()["x-content-type-options"], "nosniff");
    assert!(resp.headers().contains_key("content-security-policy"));

    env.engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pairing_issues_scoped_token() {
    let env = engine().await;
    let local = router(env.engine.clone(), ListenerKind::Local);
    let remote = router(env.engine.clone(), ListenerKind::Remote);
    let admin = env.engine.local_token();

    let (s, info) = call(
        &local,
        "POST",
        "/api/v1/devices/pairing",
        Some(&admin),
        Some(json!({ "scopes": ["read"] })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{info}");
    let code = info["code"].as_str().unwrap().to_owned();

    // Wrong code.
    let (s, _) = call(
        &remote,
        "POST",
        "/api/v1/pair",
        None,
        Some(json!({ "code": "ZZZZ-ZZZZ", "device_name": "x", "device_kind": "phone" })),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    let (s, v) = call(
        &remote,
        "POST",
        "/api/v1/pair",
        None,
        Some(json!({ "code": code, "device_name": "Phone", "device_kind": "phone" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let token = v["token"].as_str().unwrap().to_owned();
    assert_eq!(v["device"]["scopes"], json!(["read"]));
    let device_id = v["device"]["id"].as_str().unwrap().to_owned();

    // Read works, control/add/admin do not.
    let (s, _) = call(&remote, "GET", "/api/v1/tasks", Some(&token), None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = call(&remote, "POST", "/api/v1/tasks/pause-all", Some(&token), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(v["error"]["type"], "permission_denied");
    let (s, _) = call(
        &remote,
        "POST",
        "/api/v1/tasks",
        Some(&token),
        Some(json!({ "url": "http://127.0.0.1:9/a" })),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = call(&remote, "GET", "/api/v1/settings", Some(&token), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    // Archives are local-only regardless of scope.
    let (s, _) = call(
        &remote,
        "POST",
        "/api/v1/archives/list",
        Some(&token),
        Some(json!({ "path": "/tmp/x.zip" })),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // A device may forget itself; afterwards its token is dead.
    let (s, _) = call(
        &remote,
        "DELETE",
        &format!("/api/v1/devices/{device_id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(&remote, "GET", "/api/v1/tasks", Some(&token), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    env.engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn websocket_hello_and_events() {
    let env = engine().await;
    let token = env.engine.local_token();
    let server = osprey_server::start(
        env.engine.clone(),
        ServerOptions {
            socket_path: Some(env._data.path().join("osprey.sock")),
            local_tcp: Some("127.0.0.1:0".parse().unwrap()),
            remote: Some(osprey_server::RemoteOptions {
                bind: "127.0.0.1:0".parse().unwrap(),
                tls: true,
                tls_dir: env._data.path().join("tls"),
                allowed_origins: vec![],
                rate_limit_per_minute: 300,
            }),
            serve_web_ui: true,
        },
    )
    .await
    .expect("server starts");
    let addr = server.local_addr.expect("bound");
    assert!(server.remote_addr.is_some());
    assert_eq!(server.tls_fingerprint.as_deref().map(str::len), Some(64));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let sock = server.socket_path.clone().unwrap();
        let mode = std::fs::metadata(&sock).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        // Health over the Unix socket.
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut s = tokio::net::UnixStream::connect(&sock).await.unwrap();
        s.write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        assert!(out.starts_with("HTTP/1.1 200"), "{out}");
    }

    // Query tokens are accepted on the events route…
    let url = format!("ws://{addr}/api/v1/events?token={token}");
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.expect("ws connects");
    let first = ws.next().await.unwrap().unwrap();
    let hello: Value = serde_json::from_str(first.to_text().unwrap()).unwrap();
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["data"]["resync"], true);

    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"type": "ping"}).to_string().into(),
    ))
    .await
    .unwrap();

    // Adding a task produces a task_added frame carrying a TaskRow.
    let local = router(env.engine.clone(), ListenerKind::Local);
    let (s, _) = call(
        &local,
        "POST",
        "/api/v1/tasks",
        Some(&token),
        Some(json!({ "url": "http://127.0.0.1:9/ws.bin", "start": false })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let (mut got_pong, mut got_added) = (false, false);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    while !(got_pong && got_added) {
        let msg = tokio::time::timeout_at(deadline, ws.next())
            .await
            .expect("frames arrive")
            .unwrap()
            .unwrap();
        let Ok(text) = msg.to_text() else { continue };
        let Ok(v) = serde_json::from_str::<Value>(text) else { continue };
        match v["type"].as_str() {
            Some("pong") => got_pong = true,
            Some("task_added") => {
                assert!(v["seq"].as_u64().unwrap() >= 1);
                assert!(v["data"]["name"].is_string());
                assert!(v["data"].get("source").is_none(), "TaskRow, not Task");
                got_added = true;
            }
            _ => {}
        }
    }

    // …but not on ordinary routes.
    let resp = reqwest_like_get(addr, &format!("/api/v1/tasks?token={token}")).await;
    assert!(resp.starts_with("HTTP/1.1 401"), "{resp}");

    drop(ws);
    let sock = server.socket_path.clone();
    server.shutdown().await;
    if let Some(sock) = sock {
        assert!(!sock.exists(), "socket removed on shutdown");
    }
    env.engine.shutdown().await;
}

/// Minimal raw HTTP GET (avoids pulling an HTTP client into dev-dependencies).
async fn reqwest_like_get(addr: std::net::SocketAddr, path: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    out
}

/// A paired device may only save inside the folders chosen on this computer; before the fix it
/// could name any non-system directory (and, with the replace policy, overwrite user files).
#[tokio::test(flavor = "multi_thread")]
async fn remote_save_directory_is_confined() {
    let env = engine().await;
    let local = router(env.engine.clone(), ListenerKind::Local);
    let remote = router(env.engine.clone(), ListenerKind::Remote);
    let admin = env.engine.local_token();
    let (_, info) = call(
        &local,
        "POST",
        "/api/v1/devices/pairing",
        Some(&admin),
        Some(json!({ "scopes": ["read", "add"] })),
    )
    .await;
    let (_, v) = call(
        &remote,
        "POST",
        "/api/v1/pair",
        None,
        Some(json!({ "code": info["code"], "device_name": "Phone", "device_kind": "phone" })),
    )
    .await;
    let token = v["token"].as_str().unwrap().to_owned();
    let elsewhere = tempfile::tempdir().unwrap();
    let (s, _) = call(
        &remote,
        "POST",
        "/api/v1/tasks",
        Some(&token),
        Some(json!({ "url": "https://example.invalid/a.bin", "directory": elsewhere.path() })),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let inside = env._downloads.path().join("sub");
    let (s, v) = call(
        &remote,
        "POST",
        "/api/v1/tasks",
        Some(&token),
        Some(json!({ "url": "https://example.invalid/a.bin", "directory": inside })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    env.engine.shutdown().await;
}
