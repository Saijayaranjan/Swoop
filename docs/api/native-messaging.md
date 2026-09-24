# Browser extension ↔ native host protocol

The extension talks to `osprey native-host` through Chrome/Firefox Native Messaging (4-byte
little-endian length prefix + UTF-8 JSON, max 1 MiB per message). The host is a **stateless relay**
to the local API: it reads the local token from `local-api.token`, forwards requests to the Unix
socket (or loopback TCP), and streams selected events back. The extension never sees the token.

Host manifest (`app.osprey.bridge`) is installed by the macOS app (`Settings → Browser
integration → Install`) into:
- Chrome: `~/Library/Application Support/Google/Chrome/NativeMessagingHosts/app.osprey.bridge.json`
- Chromium/Brave/Edge/Arc/Vivaldi: their equivalents
- Firefox: `~/Library/Application Support/Mozilla/NativeMessagingHosts/app.osprey.bridge.json`
`allowed_origins` / `allowed_extensions` are pinned to the published extension ids.

## Messages (extension → host)

```json
{"id": 1, "type": "ping"}
{"id": 2, "type": "request", "method": "POST", "path": "/api/v1/tasks", "body": {…}}
{"id": 3, "type": "subscribe", "events": ["task_state_changed","progress","notification"], "tasks": ["…"]}
{"id": 4, "type": "unsubscribe"}
```
Responses: `{"id": n, "ok": true, "status": 200, "body": …}` or
`{"id": n, "ok": false, "error": {"type": "…", "message": "…"}}`; `{"type":"pong","version":"…","running":true}`.
Events: `{"type": "event", "event": {"type": "…", "data": …}}` (same shape as the WebSocket).

If the app is not running the host answers `{"ok":false,"error":{"type":"unavailable",
"message":"Osprey is not running"}}` and the extension offers to launch it (`open -b app.osprey.desktop`
is done by the host on `{"type":"launch"}` — macOS only).

Only `/api/v1/tasks…` and `/api/v1/media…` are relayed, with the methods GET, POST, PUT, PATCH and
DELETE. Every other group (settings, devices, automations, rules, queues, updates, plugins,
import/export, archives, …) is refused by the host, as are paths containing `..`, `//`, `%`, `#`,
backslashes or whitespace. The relay authenticates with the local token, so the allowlist is what
keeps a compromised extension away from administrative routes.
