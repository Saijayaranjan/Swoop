# Browser extension ↔ native host protocol

The extension talks to `swoop native-host` through Chrome/Firefox Native Messaging (4-byte
little-endian length prefix + UTF-8 JSON, max 1 MiB per message). The host is a **stateless relay**
to the local API: it reads the local token from `local-api.token`, forwards requests to the Unix
socket (or loopback TCP), and streams selected events back. The extension never sees the token.

## Host registration

There is no setup step. Every time the macOS app launches it writes the host manifest
(`app.swoop.bridge.json`) into the `NativeMessagingHosts` folder of each browser it finds, in the
background, and only rewrites a file whose contents changed (Settings → Browser shows the status
and has a **Reconnect browsers** button). A browser counts as installed when its profile in
`~/Library/Application Support` has been created, judged by its `Local State` file (Firefox:
`Firefox/profiles.ini`). A bare `NativeMessagingHosts` folder is not enough, since other tools
create those for browsers that aren't there.

| Browser | Profile folder | Manifest folder |
|---|---|---|
| Chrome, Chrome Beta, Chrome Canary | `Google/Chrome`, `Google/Chrome Beta`, `Google/Chrome Canary` | `<profile folder>/NativeMessagingHosts` |
| Chromium, Brave, Edge, Vivaldi | `Chromium`, `BraveSoftware/Brave-Browser`, `Microsoft Edge`, `Vivaldi` | `<profile folder>/NativeMessagingHosts` |
| Arc | `Arc/User Data` | `Arc/User Data/NativeMessagingHosts` |
| Opera | `com.operasoftware.Opera` | `Google/Chrome/NativeMessagingHosts` (Opera reads Chrome's) |
| Firefox | `Firefox` | `Mozilla/NativeMessagingHosts` |

The manifest's `path` is the running bundle's `Swoop.app/Contents/Helpers/swoop`, so moving the
app is repaired by the next launch. Browsers start that binary directly with their own arguments
(the extension origin, or Firefox's manifest path and extension id), which it recognises as a
native-host launch. `swoop native-host --install-manifest all` does the same from the command line.

The extension ids are fixed, so the manifests can allow them without asking the user:

- **Chromium** (Chrome, Edge, Brave, Arc, Vivaldi, Opera): `hbfgocpejejjhpigpanikchicoplcfjb`,
  in `allowed_origins` as `chrome-extension://hbfgocpejejjhpigpanikchicoplcfjb/`. Chrome derives
  it from the public `key` in `extensions/browser/manifests/{chrome,edge}.json`, so an unpacked
  build gets the same id on every machine. The private key is not in the repository and is not
  needed to load the extension unpacked.
- **Firefox**: the gecko id `swoop@swoop.app`, in `allowed_extensions`.

The same ids live in `CHROMIUM_EXTENSION_ID` / `FIREFOX_EXTENSION_ID`
(`crates/swoop-cli/src/commands/native_host.rs`) and `NativeMessagingInstaller`
(`apps/macos/Sources/SwoopKit/Platform/NativeMessaging.swift`); a store-assigned id, if one is
ever needed, is added next to them.

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
"message":"Swoop is not running"}}` and the extension offers to launch it (`open -b app.swoop.desktop`
is done by the host on `{"type":"launch"}` — macOS only).

Only `/api/v1/tasks…` and `/api/v1/media…` are relayed, with the methods GET, POST, PUT, PATCH and
DELETE. Every other group (settings, devices, automations, rules, queues, updates, plugins,
import/export, archives, …) is refused by the host, as are paths containing `..`, `//`, `%`, `#`,
backslashes or whitespace. The relay authenticates with the local token, so the allowlist is what
keeps a compromised extension away from administrative routes.
