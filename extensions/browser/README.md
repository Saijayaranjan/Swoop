# Osprey Browser Extension

The official Manifest V3 browser extension for [Osprey](../../docs/architecture/001-architecture-decision.md),
a production-grade download manager. One TypeScript codebase, three builds: Chrome, Firefox and
Edge.

The extension never talks to the Osprey HTTP API directly. It speaks
[Native Messaging](../../docs/api/native-messaging.md) to the `osprey native-host` binary
(`chrome.runtime.connectNative("app.osprey.bridge")`), which is a stateless relay onto the local
REST API (`docs/api/rest.md`) and event stream (`docs/api/websocket.md`). The extension never sees
the local API token. Optionally, it can instead be paired with Osprey running on another computer
(see [Remote connection](#remote-connection)).

## What you see

- **Popup** (`src/popup/`): the Osprey mark with a live connection pill (Connected / Not running
  with a one-click Launch / Offline), a speed card (download speed set large, upload and active
  count as chips, fed by `global_stats`), an "add link" field with a paste button (http, https,
  ftp and magnet links), media found on the current page, and active plus recent downloads with
  file-type tiles, slim status-coloured progress bars, speed/ETA and pause/resume/retry/cancel/show
  actions. The footer holds the **Capture downloads** switch (`intercept_downloads`), a links
  picker for the current page, Open Osprey and Settings. When Osprey can't be used, a small
  illustrated card explains why and offers the fix (launch the app, install the browser helper, or
  check the remote connection).
- **Options** (`src/options/`): a sidebar of sections — Capture, Sites, Media, Notifications,
  Connection, Shortcuts, About — each a set of grouped cards. The section lives in the URL hash so
  the popup can deep-link (`options/index.html#connection`). Changes save immediately.
- **In-page UI** (`src/content/page-ui.ts`), rendered in a closed shadow root so page CSS can't
  reach it: a small prompt after a download is captured ("Sent to Osprey" with Open Osprey and
  **Use browser instead**, then an offer to always leave that site to the browser), a "Kept in
  your browser" note when Osprey isn't running, and a sticky warning if the hand-off failed after
  the browser's copy was cancelled. Prompts never take focus, pause their timer while hovered or
  focused, and close with Esc. When enabled, a **Download** button appears over video/audio players
  (point at the player, or focus it and press Tab) and opens a panel of the page's media.

All three share the tokens in `src/shared-ui/tokens.css` (soft sky/indigo wash, frosted cards, one
system-blue accent, green/orange/red/grey status colours) and follow `prefers-color-scheme`,
`prefers-reduced-motion` and `prefers-contrast`. Icons and illustrations are built with
`createElementNS` from data (`src/shared-ui/icons.ts`, `brand.ts`) rather than parsed markup.

## Building

```sh
npm install
npm run build          # dist/chrome, dist/firefox, dist/edge
npm run build:chrome
npm run build:firefox
npm run build:edge
npm run watch           # rebuild on change, all three targets
npm run lint             # tsc --noEmit (strict)
npm test                  # node:test, pure modules only
```

`npm run build` also (re)generates `icons/*.png` when they are missing, from
`scripts/generate-icons.mjs` — a small pure-Node PNG encoder (zlib deflate + hand-rolled CRC32, no
image library) that rasterises the Osprey mark (the wing glyph on a blue tile, with supersampled
anti-aliasing) at 16/32/48/128px. `npm run icons` regenerates them on demand. The popup and options
stylesheets are bundled by esbuild so they can `@import` the shared tokens.

## Loading it unpacked

### Chrome / Edge / Brave / Vivaldi

1. `npm run build:chrome` (or `build:edge` — the two manifests are identical).
2. Open `chrome://extensions` (or the equivalent), enable **Developer mode**.
3. **Load unpacked** → select `extensions/browser/dist/chrome` (or `dist/edge`).
4. Note the extension id Chrome assigns — you'll need it for the native host manifest below.

### Firefox

1. `npm run build:firefox`.
2. Open `about:debugging#/runtime/this-firefox`.
3. **Load Temporary Add-on…** → select `extensions/browser/dist/firefox/manifest.json`.
   (Temporary add-ons are removed when Firefox restarts; for a persistent install during
   development, sign it via `web-ext sign` or use a Developer Edition/Nightly build with
   `xpinstall.signatures.required` set to `false`.)

## Installing the native messaging host (development)

In production the desktop app installs the native-messaging host manifest for you (**Settings →
Browser integration → Install**). For local development against an unpacked extension, whose id is
not the one pinned in the published host manifest's `allowed_extensions`, install it manually and
point it at your dev extension id:

```sh
osprey native-host --install-manifest chrome --extension-id <your-dev-extension-id>
osprey native-host --install-manifest firefox --extension-id osprey@osprey.app
```

This writes `app.osprey.bridge.json` to the per-browser native-messaging-hosts directory (see
`docs/api/native-messaging.md` for the exact paths) with `allowed_origins`/`allowed_extensions`
pinned to the id you pass. Without this step the extension's popup will show "Osprey not running"
even if the desktop app is open, because the browser refuses to start the host process for an
unrecognised extension id.

## How interception works, per browser

Both paths apply the same shared decision logic (`src/shared/url-utils.ts` `decideInterception`):
enabled in settings, not a never-touch scheme (`blob:`, `data:`, `chrome:`, `moz-extension:`, …),
not an excluded domain/URL pattern, not on the per-site "always use the browser" list, extension in
the configured list, and — when the size is already known — at or above `intercept_min_size`. A
download id is recorded in `storage.session` the moment it's handled, so a service-worker restart
mid-decision can never intercept (or double-forward) the same item twice.

- **Chrome / Edge** (`downloads.onDeterminingFilename`, `src/background/download-interception.ts`):
  this event fires *before* the file is written but offers no synchronous "refuse this download"
  hook, so we call the browser's own `suggest()` immediately (the UI never stalls) and, if
  interception applies, cancel + erase the item right after. Before cancelling we `ping()` the
  native host — if Osprey isn't running, we deliberately do **not** cancel (the user would lose the
  file), and instead raise a short amber badge hint.
- **Firefox** (`downloads.onCreated` + `downloads.cancel` + `downloads.erase`): Firefox does not
  implement `onDeterminingFilename`, so the item already exists (and may have started writing) by
  the time we see it; we cancel and erase it from history once we decide to take it over. Same
  reachability check and badge-hint fallback as above.

Both paths forward the referring page URL (`options.referer`) and the URL's cookies
(`browser.cookies.getAll({url})` → a `name=value; …` header string in `options.cookies`,
`src/shared/cookie-utils.ts`) so Osprey can continue a download that needs an authenticated
session.

## Media detection

- **Network-level** (`src/media-detector.ts`): `webRequest.onHeadersReceived` in observe-only mode
  (`["responseHeaders"]`, no blocking — MV3 has no blocking `webRequest`, and
  `declarativeNetRequest` cannot observe response headers, so this is passive on both browsers).
  Classifies by `Content-Type` first, URL extension as a fallback
  (`src/shared/media-classify.ts`), records size from `Content-Length`, dedups by URL, caps at 200
  entries per tab, and clears a tab's entries on top-frame navigation.
- **Page-level** (`src/content/index.ts`): scans `<a href>`, `<video>`/`<audio>`/`<source>`,
  `<img>` (large only, ≥300px in either dimension) and `link[rel~="alternate"]` playlist links, on
  demand only (`{type:"scan"}` / `{type:"scan-selection"}`). Separately, the content script shows
  the in-page UI described above; it asks the background for its settings (`get-page-ui-config`)
  only once a player is pointed at or focused, so pages without media never message it.
- The popup's **On this page** section merges both sources, deduped by URL, and enriches any
  `.m3u8` entries with real variants/resolution/estimated size via `POST /api/v1/media/detect`
  once, when the popup opens.
- **DRM**: `navigator.requestMediaKeySystemAccess` calls cannot be observed from a content script,
  so instead we flag `<video>`/`<audio>` elements whose resolved `src` is a `blob:` URL (the
  universal pattern behind MSE/EME playback) and elements that fire the `encrypted` event, both as
  "not downloadable" — their Download/Queue actions are disabled in the UI.

## Service-worker resilience

Chrome may suspend the MV3 service worker at any time. `src/background/native-port.ts` never
assumes a live native-messaging port exists: every call goes through `ensureConnected()`, which
reconnects lazily and replays the last `subscribe` request. Reconnection uses exponential backoff
(1s → 30s cap) and only keeps retrying in the background while something has actually subscribed to
live events (badge/notifications) — a plain one-off API call just reconnects on demand instead of
spinning forever. `src/background/state.ts` persists the badge count, the "popup is open" flag, and
the last ~200 already-handled download ids in `browser.storage.session` (in-memory, cleared on
browser restart, never written to disk) so a restart doesn't lose that state or double-handle an
in-flight download.

## Permissions rationale

| Permission | Why |
|---|---|
| `downloads` | Read/cancel/erase browser downloads to intercept them (`downloads.onDeterminingFilename`/`onCreated`, `.cancel`, `.erase`). |
| `contextMenus` | "Download with Osprey" on links/images/video/audio/page/selection. |
| `nativeMessaging` | Talk to `osprey native-host` — the extension's only channel to Osprey. |
| `storage` | Settings (`storage.local`) and session-scoped resilience state (`storage.session`). |
| `notifications` | Completion/failure toasts when the popup is closed. |
| `webRequest` | Observe-only response headers for network-level media detection. No blocking, no header modification. |
| `tabs` | Resolve the active tab's id/URL/title for interception hints, the "send page URL" command, and messaging the content script. |
| `activeTab` | Least-privilege companion to `tabs` for user-invoked actions (context menu, toolbar). |
| `cookies` | Forward the intercepted URL's cookies to Osprey (`options.cookies`) so authenticated downloads keep working — never sent anywhere but the local native host. |
| `host_permissions: ["<all_urls>"]` | Required by `webRequest.onHeadersReceived` to observe response headers on arbitrary sites for media detection, and by the content script to scan any page. No page content is ever sent anywhere except a same-machine native-messaging relay. |
| `commands` | `Alt+Shift+D` — send the current page URL to Osprey. |

## Remote connection

Options → **Connection** → *Another computer* pairs the extension with an Osprey whose remote
listener is on (docs/api/rest.md): enter its `https://` address and a one-time pairing code from
that Osprey (`POST /api/v1/pair`), or paste a device token. The token is stored under its own
`storage.local` key, never in the settings object, and where the browser supports it (Chrome/Edge)
`storage.local` is restricted to trusted contexts; content scripts never read storage themselves. Requests then go over HTTPS with the bearer
token (`src/background/remote-client.ts`) and live events over the WebSocket stream; plain `http:`
is refused except for loopback addresses. Captured downloads, including their cookies, are sent to
that computer. *This computer* (the default) switches back to native messaging.

## Message hardening

Content scripts run inside arbitrary pages, so the background only accepts a small set of request
types from them (`CONTENT_SCRIPT_MESSAGE_TYPES` in `src/shared/messages.ts`: page UI config, the
sender tab's detected media, quick download, launch, and in-page prompt actions). Raw API requests,
pairing and everything else are accepted from the extension's own pages only.

## Privacy

Nothing the extension sees — page content, detected media, cookies, download URLs — leaves the
machine, unless you pair it with a remote Osprey (above), in which case it goes only to that
Osprey over HTTPS. Otherwise the only outbound channel is native messaging to the locally-installed
`osprey native-host` process, which itself only relays to the local Osprey API over a Unix socket /
loopback TCP. The extension stores only its own settings and site exclusions (`browser.storage.local`) and a
small amount of session-scoped resilience state (`browser.storage.session`, cleared on browser
restart). There is no analytics, telemetry, or remote code — every script shipped is bundled from
this repository at build time (`npm run build`).

## Internationalisation

All user-visible strings live in `_locales/<lang>/messages.json`, read exclusively through
`src/shared/i18n.ts`. English is authoritative; Hindi (`hi`) and Tamil (`ta`) are full translations.
All three are generated from one source of truth by `scripts/build-locales.mjs` (`npm run locales`)
so they can never drift out of key-sync: a key without a translation falls back to English with its
`description` marked `[fallback: needs translation]`. Substitutions are written `$1` in the source
and emitted as named placeholders.

## Tests

`npm test` runs Node's built-in test runner directly against the TypeScript sources (Node 22+
strips types natively — no ts-node, no build step). Coverage focuses on pure, browser-API-free
modules: interception decisions and exclusion matching (`url-utils`), media classification
(`media-classify`), cookie header building (`cookie-utils`), settings persistence against a fake
`storage.local`-shaped object (`settings`), and native-messaging request/response correlation,
timeouts, and event fan-out against a fake transport (`api-client`, `native-protocol`), byte and
speed formatting (`format`), and remote URL / pairing-code validation (`remote`).

## Known limitations

- **Firefox interception timing**: because Firefox has no `onDeterminingFilename` equivalent, a
  very small/fast download may finish writing to disk before `onCreated` fires and we cancel it;
  the partial/complete browser-side file is erased from the download shelf either way, but a
  fraction of a second of disk I/O for that copy is unavoidable on Firefox.
- **No tab-to-download linking in the `downloads` API**: neither browser exposes the originating
  tab id on a `DownloadItem`, so the "tab title" hint forwarded with an intercepted download is a
  best-effort match against `tabs.query({url: item.referrer})` — it can miss (e.g. downloads
  triggered from an iframe whose URL differs from the top-level tab).
- **Detected-media list is not persisted across a service-worker suspension**: `media-detector.ts`
  keeps its per-tab store in memory (by design — network activity naturally repopulates it, and
  persisting up to 200 entries × every tab to `storage.session` on every response would be
  wasteful); a worker that was suspended and woken again for an unrelated reason starts that tab's
  list fresh from whatever responses arrive afterwards, until the user reloads the page.
- **`action.openPopup()` for bulk link picking**: "Download all links on page…" and "…in
  selection" try `browser.action.openPopup()` (Chrome 99+ / Firefox 118+) and fall back to opening
  the popup HTML in a normal tab when it's unavailable or rejected (it requires a very recent user
  gesture in some engines); either way the gathered links land in `storage.session` for the popup's
  links picker to pick up.
- **HLS variant estimated sizes** depend entirely on what `POST /api/v1/media/detect` returns;
  the extension does not parse playlists itself.
- **Paste button**: reading the clipboard needs a permission the extension doesn't request, so
  when the browser refuses, the popup focuses the field and asks for ⌘V / Ctrl+V instead.
- **In-page prompts target the active tab**: the `downloads` API doesn't say which tab started a
  download, so the prompt appears in the focused window's active tab (nothing is shown on pages
  without a content script, such as the browser's own pages).
- **Remote Osprey with its own certificate**: the browser must trust the remote listener's TLS
  certificate; open the address in a tab once and accept it before pairing.
