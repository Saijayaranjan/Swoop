# Osprey HTTP API (v1)

One API, three listeners:

| Listener | Default | Purpose | Auth |
|---|---|---|---|
| Unix socket `~/Library/Application Support/Osprey/osprey.sock` (mode 0600) | always on while the app/daemon runs | CLI, native-messaging host | peer must be the same uid **and** present the local token |
| Loopback TCP `127.0.0.1:41779` | on for headless/Windows, **off** on the macOS app by default | CLI/extension host where Unix sockets are unavailable | local token |
| Remote TCP `0.0.0.0:41780` (TLS) | **off** | phones, tablets, other computers, browsers | paired-device bearer token with scopes |

Base path: `/api/v1`. JSON everywhere (`Content-Type: application/json`). Every response is either the
resource or `{"error": {"type": "not_found" | "validation" | "conflict" | "permission_denied" |
"storage" | "engine" | "unavailable" | "internal" | "unauthorized" | "rate_limited", "message": "…"}}`
with a matching status (404, 400, 409, 403, 500, 500, 503, 500, 401, 429).

## Authentication

- `Authorization: Bearer <token>` on every request (also accepted as the `token` query parameter
  **only** for `GET /api/v1/events` WebSocket upgrades and `/api/v1/tasks/{id}/file` downloads).
- Local token: random 256-bit, stored in `local-api.token` (0600). Grants `admin` scope.
- Device tokens: issued by pairing; SHA-256 hashed at rest; scopes `read`, `add`, `control`, `admin`.
- Origin checks: browser requests must send an `Origin` in `settings.remote.allowed_origins` or the
  extension origin (`chrome-extension://…`, `moz-extension://…`) — otherwise 403.
- Rate limit: `settings.remote.rate_limit_per_minute` per token (429 with `Retry-After`).
- Pairing brute force: `max_failed_attempts` per IP → lockout `lockout_minutes`.
- Every remote request is written to the audit log (device, ip, action, target, success).

Scope required per route group: **R** = read, **A** = add, **C** = control, **X** = admin.

## Pairing (remote listener only)

```
POST /api/v1/pair                       {"code":"ABCD-EFGH","device_name":"iPhone","device_kind":"phone"}
  → 200 {"device": Device, "token": "…"}   (token shown once)
  → 401 {"error":{"type":"unauthorized"}}  (wrong/expired code; counts toward lockout)
```
Pairing codes are created from the desktop UI (`EngineApi::start_pairing`), are 8 characters
(`ABCD-EFGH`, unambiguous alphabet), single-use, and expire after 2 minutes. The QR code encodes
`osprey://pair?host=<ip>&port=<port>&fp=<sha256 of TLS cert>`; the code is typed separately.

## Engine

```
GET  /api/v1/info                 R  → EngineInfo
GET  /api/v1/stats                R  → GlobalStats
GET  /api/v1/dashboard            R  → Dashboard
GET  /api/v1/settings             X  → Settings
PUT  /api/v1/settings             X  Settings → Settings
POST /api/v1/environment          X  EnvironmentSnapshot → 204     (platform probe pushes conditions)
POST /api/v1/traffic-mode         C  {"mode":"browsing"} → 204
POST /api/v1/limits               C  {"download":0,"upload":0} → 204
POST /api/v1/optimize             C  → Settings
GET  /api/v1/disk?path=…          R  → DiskInfo
GET  /api/v1/logs?limit=200&level=warn   X → [string]
```

## Tasks

```
POST /api/v1/tasks/probe          A  NewTaskRequest → ProbeResult
POST /api/v1/tasks                A  NewTaskRequest → AddTaskResult          (201)
POST /api/v1/tasks/batch          A  [NewTaskRequest] → [AddTaskResult]
GET  /api/v1/tasks                R  query: text, state (repeatable), kind, queue_id, category_id,
                                     domain, tag, smart, sort, desc, limit, offset → TaskPage
GET  /api/v1/tasks/rows           R  same query → [TaskRow]  (cheap list for UIs)
GET  /api/v1/tasks/{id}           R  → Task
PATCH /api/v1/tasks/{id}          C  TaskPatch → Task
DELETE /api/v1/tasks/{id}?delete_file=false   C → 204
POST /api/v1/tasks/remove         C  {"ids":[…],"delete_file":false} → {"removed":n}
POST /api/v1/tasks/{id}/start|pause|resume|restart|retry|cancel|redownload|verify|retry-segments  C → Task
POST /api/v1/tasks/{id}/retry-from-source   C {"url": "…"?} → Task
POST /api/v1/tasks/{id}/duplicate C  → Task
POST /api/v1/tasks/{id}/resolve-duplicate   C {"policy":"rename"} → Task
POST /api/v1/tasks/{id}/limit     C  {"download":…,"upload":…} → Task
POST /api/v1/tasks/{id}/connections C {"connections":8} → Task
POST /api/v1/tasks/{id}/priority  C  {"priority":"high"} → Task
POST /api/v1/tasks/reorder        C  {"ids":[…],"after":id|null} → 204
GET  /api/v1/tasks/{id}/log?limit=200        R → [TaskLogEntry]
GET  /api/v1/tasks/{id}/diagnostics          R → TaskDiagnostics
GET  /api/v1/tasks/{id}/diagnostics.txt      R → text/plain
GET  /api/v1/tasks/{id}/file                 R → the completed file (Content-Disposition), 409 if not complete
POST /api/v1/tasks/pause-all|resume-all|retry-failed|clear-completed   C → {"count":n}
```

### Torrents
```
GET  /api/v1/tasks/{id}/peers               R → [PeerInfo]
PUT  /api/v1/tasks/{id}/files               C [FileSelection] → Task
POST /api/v1/tasks/{id}/sequential          C {"sequential":true} → Task
PUT  /api/v1/tasks/{id}/seeding             C SeedingLimits → Task
POST /api/v1/tasks/{id}/trackers            C {"trackers":[…]} → Task
DELETE /api/v1/tasks/{id}/trackers          C {"tracker":"…"} → Task
POST /api/v1/tasks/{id}/trackers/enable     C {"tracker":"…","enabled":false} → Task
POST /api/v1/tasks/{id}/reannounce          C → 204
POST /api/v1/trackers/refresh               X → {"applied":n}
```

### Media
```
POST /api/v1/media/detect        A {"url":"…","page_url":"…"} → DetectedMedia
```

## Queues, categories, rules, schedules, automation, recipes
Uniform CRUD (`R` for GET, `X` for mutations; queue pause/resume is `C`):
```
GET    /api/v1/queues            GET /api/v1/queues/summaries
POST   /api/v1/queues            Queue → Queue (201)
PUT    /api/v1/queues/{id}       Queue → Queue
DELETE /api/v1/queues/{id}?move_to=<queue_id>
POST   /api/v1/queues/{id}/pause | resume
POST   /api/v1/queues/reorder    {"ids":[…]}
GET/POST/PUT/DELETE /api/v1/categories[/{id}]
GET/POST/PUT/DELETE /api/v1/rules[/{id}]         POST /api/v1/rules/test  RuleSubject → [{rule, actions}]
GET/POST/PUT/DELETE /api/v1/schedules[/{id}]
GET/POST/PUT/DELETE /api/v1/automations[/{id}]   GET /api/v1/automations/runs?id=&limit=
POST   /api/v1/automations/{id}/run  {"task_id":"…"} → AutomationRun
GET/POST/PUT/DELETE /api/v1/recipes[/{id}]       POST /api/v1/recipes/{id}/apply NewTaskRequest → AddTaskResult
```
**Remote clients may not create or modify automations containing `run_command`, `run_shell`,
`run_apple_script` or `run_sandboxed_script` actions, and `grant consent` is not exposed over HTTP at
all** (403 `permission_denied`). Consent is granted only through the desktop UI (FFI).

## History
```
GET    /api/v1/history           R  query = HistoryQuery fields → [HistoryEntry]
GET    /api/v1/history/count     R
POST   /api/v1/history/delete    C  {"ids":[…]}
DELETE /api/v1/history           X
```

## Devices (admin)
```
GET    /api/v1/devices
POST   /api/v1/devices/pairing   {"scopes":["read","control"]} → PairingInfo
DELETE /api/v1/devices/pairing
DELETE /api/v1/devices/{id}      (revoke)
PATCH  /api/v1/devices/{id}      {"name":"…"}
GET    /api/v1/audit?limit=200
```

## Site grabber, archives, import/export, updates, plugins
```
POST /api/v1/grabber             A GrabberOptions → GrabberSession (201)
GET  /api/v1/grabber             R → [GrabberSession]
GET  /api/v1/grabber/{id}        R → GrabberSession
DELETE /api/v1/grabber/{id}      C
POST /api/v1/grabber/{id}/add    A {"urls":[…],"request":NewTaskRequest} → [AddTaskResult]
POST /api/v1/archives/list       R {"path":"…"} → ArchiveListing         (local listeners only)
POST /api/v1/archives/extract    C {"path":"…","entries":[…]|null,"destination":"…"} → {"extracted":n}  (local only)
GET  /api/v1/export?tasks=true&history=false   X → ExportBundle
POST /api/v1/import              X {"bundle":ExportBundle,"options":ImportOptions} → ImportReport
POST /api/v1/updates/check       X → UpdateInfo
POST /api/v1/updates/download    X → {"path":"…"}
GET  /api/v1/plugins             X   POST /api/v1/plugins/{id}/enable {"enabled":bool,"permissions":[…]}   DELETE /api/v1/plugins/{id}
```

## Health
`GET /healthz` (no auth, all listeners) → `{"status":"ok","version":"…"}`.

## Web UI
The remote listener serves the embedded web UI at `/` (static files, SPA fallback to `index.html`).
The local listeners serve it too so `http://127.0.0.1:41779/` works for troubleshooting.
