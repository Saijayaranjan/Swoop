# swoop-server

REST + WebSocket front door over `swoop_services::EngineApi`, plus the embedded remote web UI.
Contract: `docs/api/rest.md`, `docs/api/websocket.md`; security rationale:
`docs/security/threat-model.md`.

```rust
let engine = swoop_services::bootstrap::start(config).await?;
let server = swoop_server::start(engine.clone(), swoop_server::ServerOptions {
    socket_path: Some(paths.socket()),
    local_tcp: Some("127.0.0.1:41779".parse()?),
    remote: Some(swoop_server::RemoteOptions { bind: "0.0.0.0:41780".parse()?, ..Default::default() }),
    serve_web_ui: true,
}).await?;
println!("TLS fingerprint: {:?}", server.tls_fingerprint);
// …
server.shutdown().await;
```

`swoop_server::router(engine, ListenerKind::Local | Remote)` returns the bare `axum::Router`
(for tests or custom listeners).

## Listeners

| Listener | Notes |
|---|---|
| Unix socket | Bound in a private 0700 staging directory, chmod 0600, then moved into place. Stale sockets are removed; a live one (another instance) is an error; a non-socket file is never touched. Peers with a different uid are dropped at accept time. |
| Loopback TCP | Refused unless the address is loopback. |
| Remote | Optional. TLS with a self-signed certificate (rcgen) persisted as `tls_dir/cert.pem` + `key.pem` (0600) and reused; `ServerHandle::tls_fingerprint` is the lower-case hex SHA-256 of the DER certificate. Plain HTTP only if `tls: false` (logged as a warning). |

## Security model (summary)

- **Auth**: `Authorization: Bearer`. `?token=` is accepted only on `GET /api/v1/events` and
  `GET /api/v1/tasks/{id}/file`. The local token is admin **on local listeners only**; on the remote
  listener it is rejected. Paired-device tokens work on every listener with their scopes.
- **Scopes**: one table (`src/auth.rs::policy`) maps every route to R/A/C/X; unknown routes need
  admin. `DELETE /devices/{id}` is also allowed for a device revoking itself.
- **Remote restrictions** (apply to every caller that is not the local token on a local listener):
  no archive routes; no creating/updating/importing automations with `run_command`, `run_shell`,
  `run_apple_script`, `run_sandboxed_script`; no manual runs of such automations; no rules/recipes
  linking to them; `open_when_done` is forced off; consent granting has no HTTP route at all.
  `platform_action`/`settings_changed` events are never streamed to them, device/pairing events
  only to admin devices, and the pairing code is blanked.
- **Origin**: requests with an `Origin` must match the server's own origin, an extension scheme
  (`chrome-extension://`, `moz-extension://`, `safari-web-extension://`), `allowed_origins`, or
  `settings.remote.allowed_origins`; otherwise 403. Allowed cross-origin callers get CORS headers.
- **Rate limit**: token bucket per device (per IP for `/pair`) on the remote listener → 429 +
  `Retry-After`. Failed-token/pairing lockouts are enforced by the engine.
- **Limits**: 1 MiB JSON bodies; 10 MiB for task add/probe/batch, recipe apply, grabber add and
  import; WebSocket frames 64 KiB.
- **Audit**: every remote API request is recorded via `EngineApi::record_audit` (method, route
  template, path without query, status).
- **Headers**: CSP (`default-src 'self'; connect-src 'self' ws: wss:; frame-ancestors 'none'…`),
  `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `X-Frame-Options: DENY`,
  `Cross-Origin-Resource-Policy: same-origin`; API responses are `Cache-Control: no-store`.

## WebSocket

`hello` on connect (`seq` 0), then per-connection monotonically increasing `seq`.
`high_volume=false` and `tasks=a,b` query filters; `{"type":"subscribe","tasks":[…],"high_volume":bool}`
changes them; `{"type":"ping"}` → `{"type":"pong"}`. Server pings every 30 s and closes after 90 s
without any inbound frame; broadcast lag produces `{"type":"lagged","data":{"missed":n}}`.
`task_added`/`task_updated` carry a `TaskRow`. The stream closes when the caller's device is revoked
or the server shuts down.

## Web UI

`web/remote/dist` is embedded with `rust-embed` (`allow_missing`), with SPA fallback to
`index.html`. If the bundle was not built, `/` serves a short page explaining
`cd web/remote && npm install && npm run build`.

## Tests

`cargo test -p swoop-server` — auth, task CRUD, pairing + scopes, origin checks, WebSocket
hello/pong/`task_added`, TLS certificate persistence, rate limiter, scope table.
