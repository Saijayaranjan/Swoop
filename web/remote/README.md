# Swoop remote web UI

A small Preact + TypeScript single-page app that lets a phone, tablet, or another computer
control Swoop over its remote REST/WebSocket API. It is built to a static `dist/` bundle and
embedded directly into the `swoop-server` binary, so it ships with zero runtime dependencies
beyond what a browser already provides.

## Requirements

- Node.js >= 23.6 (for `node --test`'s native TypeScript support) — Node 26 is what this project
  was built and tested against.
- npm >= 11.

## Scripts

```
npm install        # esbuild, typescript, preact — nothing else
npm run build       # -> dist/index.html, dist/main-<hash>.js, dist/main-<hash>.css
npm run dev         # esbuild watch + dev server on http://localhost:5173
npm run lint         # tsc --noEmit, strict
npm test             # node --test src (router, rev-merge, formatters, pairing-code, windowed-list, API client)
```

`npm run build` fails the build if the gzipped total of `index.html` + JS + CSS exceeds 150 KB,
so the size budget is enforced automatically rather than by convention.

## How the server embeds `dist/`

`crates/swoop-server` (axum) embeds the contents of `web/remote/dist/` at compile time with
[`rust-embed`](https://docs.rs/rust-embed) (see `docs/architecture/001-architecture-decision.md`,
"Remote web UI"). The SPA is served at `/` on every listener (the remote TLS listener, and the
local loopback/Unix-socket listeners for troubleshooting), with a catch-all fallback to
`index.html` for client-side routes (`#/downloads/<id>`, etc. — the router uses hash paths
specifically so no server-side route table is needed).

Build the UI (`npm run build` in this directory) before building `swoop-server`/`swoop-cli` so
`dist/` exists for the embed macro to pick up; the exact wiring (embed struct, fallback handler)
lives in `crates/swoop-server/src`.

## Pairing walkthrough

1. On the desktop app (or headless daemon admin UI), start pairing: this calls
   `EngineApi::start_pairing`, which shows an 8-character code (`ABCD-EFGH`, unambiguous
   alphabet) and a QR code encoding `swoop://pair?host=<ip>&port=<port>&fp=<sha256 of cert>`.
   The code is single-use and expires after 2 minutes.
2. Open the remote UI on the other device — either by scanning the QR code (which opens
   `https://<host>:<port>/?token=...` for a direct handoff, if the platform layer includes one)
   or by browsing to the server's address directly.
3. With no token in `localStorage`, the app shows the **Pair** screen. Type the code (it
   auto-formats to `ABCD-EFGH` as you type) and a device name (defaulted from the user agent,
   editable), and confirm the TLS fingerprint hint shown matches the server's, especially on an
   untrusted network.
4. Submitting calls `POST /api/v1/pair`; on success the returned bearer token and device record
   are stored in `localStorage` and the app switches to the main shell. A `401` from *any*
   subsequent request (expired/revoked token) clears local auth and returns to this screen.
5. **Settings → Forget this device** clears the local token (and best-effort revokes it via
   `DELETE /api/v1/devices/{id}`), returning to the Pair screen.

## Architecture notes

- **No state-management dependency.** `src/state/store.ts` is a ~20-line get/set/subscribe
  store; `src/state/useStore.ts` is the one-hook binding to Preact. Feature stores
  (`auth.ts`, `settings.ts`, `tasks.ts`, `toast.ts`) are small modules built on top of it.
- **Router.** `src/router/router.ts` exports pure `parseHash`/`buildHash` functions (unit
  tested) plus a small browser-integrated `Router` class. Hash paths (`#/downloads`,
  `#/downloads/<id>`, `#/add`, `#/dashboard`, `#/speed`, `#/settings`) need no server route table,
  which matters because the same `dist/` is served unmodified from three different listeners.
- **Live updates.** `src/state/ws.ts` owns the WebSocket connection to `/api/v1/events`,
  reconnects with capped exponential backoff, and resyncs the full task table
  (`GET /tasks/rows`) on `hello`, `lagged`, or a detected `seq` gap. `task_added`/`task_updated`
  replace a row outright; `progress` batches are merged only when their `rev` is strictly newer
  than the last one applied for that task (`src/state/taskMerge.ts`, unit tested) — this mirrors
  the documented contract in `docs/api/websocket.md` even though `TaskRow` itself carries no
  `rev` field (the client tracks a shadow "last applied progress rev" per task for exactly this
  purpose). Because the "high-volume events" setting can suppress `global_stats` entirely, the
  header's speed readout is also fed by a 4 s `GET /stats` poll, and whichever source has the
  newer `at` timestamp wins — so the download/upload speed is always live regardless of that
  toggle.
- **Windowed list.** `src/components/WindowedList.tsx` is a small from-scratch virtualised list;
  the index/padding maths live in `src/utils/windowedList.ts` as a pure function so they're unit
  tested without a DOM.
- **i18n.** `src/i18n/en.ts` is the complete, authoritative string table. `hi.ts` / `ta.ts` are
  deliberately partial dictionaries — real Hindi and Tamil translations for the ~40+ most common
  strings (navigation, task actions/states, filters, common dialog copy) — merged over `en.ts` in
  `src/i18n/index.ts` so every key always resolves to real text, never a placeholder. Long-tail
  strings (the 41 `ErrorKind` explanations, less common settings copy) fall back to English until
  someone translates them; `npm test` asserts the translated dictionaries are actually different
  from English (not silently copy-pasted) and contain no `TODO`-style placeholders.
- **API client.** `src/api/client.ts` is a thin wrapper: every call is same-origin
  (`/api/v1/...`), attaches the bearer token, and maps non-OK responses / network failures /
  unparsable bodies to one `ApiError` shape (`src/api/errors.ts`) that the UI branches on. A
  `401` anywhere calls the client's `onUnauthorized` hook, which clears auth app-wide.
- **Task actions respect the state machine.** `src/utils/taskState.ts` mirrors
  `crates/swoop-domain/src/state.rs`'s `can_pause`/`can_resume`/etc. so the row menu and detail
  view only ever offer actions the server will accept.

## Testing

`npm test` runs `node --test src`, which recursively picks up every `*.test.ts` file. Node's
native TypeScript type-stripping runs the tests directly with no build step — the one constraint
that comes with that is avoiding TS syntax that isn't pure type-erasure (constructor parameter
properties, `enum`, decorators), which the whole codebase avoids for consistency. Ambient types
for `node:test`/`node:assert/strict` live in `src/types/node-test.d.ts` (hand-written, so the
project doesn't need `@types/node` as a dependency).

Covered: the router's hash parsing, the rev-based progress/task merge logic, the pairing-code
auto-formatter, `formatBytes`/`formatEta`/`formatPercent`/`formatRelativeTime`, the windowed
list's range maths, the i18n merge/fallback behaviour, and the API client's error mapping
(with `fetch` mocked — no network access in tests).

## Known limitations

- **No offline queueing.** Actions issued while disconnected fail immediately with a toast
  rather than being queued for replay on reconnect.
- **Peer/log polling only while the relevant Detail tab is open** (peers every 5 s, log once on
  tab open) — there's no push channel for those yet, matching the REST API (no WS event carries
  peer lists or full log entries, only `task_log` for the most recent line).
- **Device rename requires `admin` scope** on the paired token (`PATCH /api/v1/devices/{id}`);
  a device paired with narrower scopes will see the "Save" action fail with a permission-denied
  toast. This mirrors the server's own authorization model rather than working around it.
- **No drag-to-reorder** in the Downloads list (`POST /tasks/reorder` is not wired up); sorting
  is available, manual reordering is not.
- **Categories, schedules, rules, automations, recipes, site grabber, history, and multi-file
  export/import** are out of scope for this remote-control surface (the brief's view list is
  Downloads/Detail/Add/Dashboard/Speed/Settings) and have no UI here, even though the REST API
  exposes them.
- **Hindi/Tamil cover the ~40 most common strings**, not the full catalogue (see "i18n" above);
  everything else reads in English under those locales until translated.
