# ADR-001 — Osprey architecture decision

**Status:** accepted (2026-09-17) · **Product name:** Osprey (original; no competitor naming/branding)

## 1. Decision in one paragraph

Osprey is a **Rust core** (download engine, BitTorrent, media/HLS, site grabber, task/queue/scheduler
services, SQLite persistence, automation, REST/WebSocket API, CLI, headless daemon) with a
**fully native SwiftUI + AppKit macOS application** that links the core **in-process through
UniFFI**. The same core binary serves as the headless server (`osprey server`), the CLI (`osprey …`),
and the browser Native-Messaging host (`osprey native-host`). Windows gets a native WinUI 3 shell
over the same core later; Linux/NAS use the headless server plus the embedded web UI.

## 2. Options evaluated

| Option | Verdict | Why |
|---|---|---|
| **A. Rust core + native SwiftUI/AppKit (macOS) + native WinUI 3 (Windows)** | **chosen** | Best UX on each platform; one authoritative engine; UI never blocks the engine (separate Tokio threads); smallest memory and fastest startup of the viable options. |
| B. Rust core + Tauri 2 + React | rejected for the *primary* UI | WKWebView adds ~150 MB and ~300 ms startup, web tables are not native tables (no native selection/type-select/VoiceOver table semantics), Quick Look / Finder tags / menu-bar popovers / notification actions all need objc bridging that produces a second-class result. Kept as the pattern for the **remote web UI** where a browser *is* the platform. |
| C. Electron | rejected | Worst memory/startup; no advantage over B. |
| D. Fully native everything (Swift engine on macOS, C# engine on Windows) | rejected | Duplicate networking/torrent/persistence logic; guaranteed behavioural drift between platforms; violates "single authoritative implementation". |
| E. Rust core + Flutter / Compose Multiplatform | rejected | Non-native look and accessibility on both desktops; large runtime. |
| F. Out-of-process daemon + thin UI (separate daemon model) | rejected as the *default* | Adds JSON serialisation of every progress event, process supervision, and UI latency for the common case. The daemon still exists (headless mode, same code) — it is simply not the only path. Crash isolation is achieved instead by keeping the Rust core panic-free at the FFI boundary (`catch_unwind`) and by durable persistence. |
| G. C++ core (libtorrent + libcurl) | rejected | Memory safety, a Boost build on every platform, and a far weaker async story than Tokio for a product that is essentially all I/O. |

### Scoring against the 30 criteria (summary)
Option A wins or ties on 27/30. It loses on **build complexity** (two UI codebases over time) and
**packaging** (SwiftPM + script bundling without Xcode). It ties with B on cross-platform
consistency of *engine behaviour* (identical core) and beats B on every UX/native criterion.

## 3. Technologies and why

| Layer | Choice | Reason | Alternatives rejected |
|---|---|---|---|
| Async runtime | `tokio` 1.x (MIT) | Industry standard; multi-threaded scheduler; needed by every network crate below | async-std (less ecosystem) |
| HTTP client | `reqwest` 0.13 + `rustls` (MIT/Apache) | HTTP/1.1 + HTTP/2, proxies (HTTP/HTTPS/SOCKS5), cookies, redirects, per-download `Client` so segment connections are not multiplexed unless we want them to be; rustls avoids OpenSSL on every platform | hyper directly (too low-level for cookies/redirect/proxy), curl bindings (C dep) |
| FTP/FTPS | `suppaftp` 12 (async + rustls) | Resume (`REST`), `MLSD`/`LIST`, TLS | async_ftp (unmaintained since 2021) |
| BitTorrent | `librqbit` 9 (Apache-2.0) | Pure Rust, Tokio-native, DHT/PEX/UDP+HTTP trackers/private torrents/IPv6/uTP/rate limits/file selection/magnet resolve | libtorrent-rasterbar (C++/Boost bridge, Windows pain, no async integration) |
| Persistence | `rusqlite` (bundled SQLite, WAL) | Fastest, zero external deps, synchronous access from one dedicated writer thread is simpler and safer than an async pool for a single-process store | sqlx (async overhead, compile-time macros against a DB file), sea-orm (heavy) |
| Hashing | `sha1`, `sha2`, `blake3`, `md5` (opt-in) | Checksum verification; BLAKE3 for fast internal duplicate fingerprints | — |
| FFI | `uniffi` 0.32 (MPL-2.0) | Type-safe Swift (and later C#) bindings, async support, callback interfaces for events; proc-macro mode (no UDL) | hand C ABI + JSON (untyped), swift-bridge (Swift only) |
| HTTP server | `axum` 0.8 + `tower-http` | Same tower ecosystem as reqwest/hyper; WebSocket built-in | actix (separate runtime model), warp |
| TLS for remote | `rustls` + `rcgen` self-signed on first enable | No OpenSSL; fingerprint shown in pairing QR | — |
| Auth | random 256-bit tokens hashed with `argon2`; pairing codes are 8-char one-time, 2-minute TTL | brute-force resistant; codes never stored in plaintext | JWT (no benefit for a local device list) |
| Secrets | macOS Keychain (Swift `Security.framework`); `keyring` crate for headless | never plaintext | — |
| HTML parsing (grabber) | `lol_html` (streaming, BSD-3) + `url` | Streaming rewriter handles huge pages with bounded memory | scraper (DOM in memory) |
| Metalink / HLS | `quick-xml`, hand-written M3U8 parser | Small, exact | — |
| CLI | `clap` 4 | Standard; `--json` output on every command | — |
| Logging | `tracing` + JSON file layer with redaction | Structured, task/event IDs | — |
| macOS UI | SwiftUI (macOS 14+) over AppKit where needed (`NSTableView`-backed `Table`, `NSMenu`, `QLPreviewPanel`, `NSWorkspace`) | Native menus, inspector, menu-bar extra, accessibility, Retina, appearance | — |
| Extension | WebExtension MV3 (TypeScript, esbuild) | One codebase for Chrome/Chromium/Edge/Firefox with per-browser manifest | — |
| Remote web UI | Preact + TypeScript (MIT), embedded into the server binary with `rust-embed` | 4 KB runtime; serves phones/tablets/NAS | — |
| Updates | Own updater: JSON appcast, **Ed25519**-signed archives, downloaded by Osprey's own engine, verified before install, previous bundle retained for rollback | Sparkle needs Xcode-style framework embedding that this host cannot do; own updater dogfoods the engine | Sparkle |

## 4. Shared vs platform-specific

**Shared (Rust, `crates/`)** — everything that has no UI: domain model & state machine, HTTP/FTP/
torrent/HLS engines, adaptive segmentation, rate limiting, queues, scheduler *logic*, rules,
automation engine, history, duplicate detection, disk-space checks, diagnostics & health score,
persistence & recovery, REST/WS API, pairing/auth, CLI, native-messaging relay, site grabber,
archive inspection, import/export, update check/verify, log redaction.

**macOS-native (Swift, `apps/macos/`)** — windows, table, inspector, menu bar extra, Dock badge,
notifications (with actions), Finder reveal & tags, Quick Look, drag & drop, Services, URL scheme /
`.torrent` / `magnet:` handlers, Keychain, login item, power assertion (no sleep during transfers),
battery/AC & network-path observers (fed to the shared scheduler as *conditions*), quarantine xattr,
Settings window, accessibility, keyboard shortcuts, update *install/relaunch*.

**Windows-native (future, `apps/windows/`)** — WinUI 3 equivalents of the above over the same
`osprey-ffi` surface (UniFFI also emits C#/Kotlin/Python bindings).

## 5. IPC strategy

1. **Desktop app ↔ core:** in-process UniFFI calls. Commands are synchronous-fast (they enqueue
   into the engine) or `async`. Events flow **core → app** through a UniFFI callback interface;
   progress is coalesced by the core into ≤4 batches/second regardless of task count.
2. **CLI / extension host / remote clients ↔ running instance:** the local API — a Unix-domain
   socket (`~/Library/Application Support/Osprey/osprey.sock`, mode 0600) plus loopback TCP. Auth
   for local callers is a random token in a 0600 file; the extension never sees it (the native
   host reads it). Remote access is a separate listener that is **off by default** and TLS-only.
3. **Browser extension ↔ native host:** Chrome/Firefox Native Messaging (length-prefixed JSON on
   stdio). The host is `osprey native-host`, a stateless relay to the local API.

## 6. Persistence strategy

SQLite (WAL, `synchronous=NORMAL`, foreign keys on) in the app-support directory; versioned
migrations via `user_version`. Every task, queue, category, rule, schedule, automation, device and
history row is durable. In-progress HTTP downloads keep a **segment map** with a *committed*
watermark per segment that is only advanced after data has been flushed to disk; on restart the
engine trusts the committed watermark and re-fetches anything past it. Files are written to
`<name>.osprey-part`, verified, then renamed atomically. Torrent state is persisted by librqbit's
session file plus our task row. Settings are a typed struct stored as JSON in one row with schema
versioning. Export/import is JSON.

## 7. Security boundaries

- **Untrusted input:** URLs, HTTP headers, filenames (`Content-Disposition`), HTML, torrent
  metadata, playlist files, archive listings, extension messages, remote API bodies, import files.
  All pass through validators in `osprey-runtime::safety` (path traversal, control chars, reserved
  names, length, symlink escape, absolute-path rejection).
- **Trust boundary 1 — FFI:** `catch_unwind` at every exported function; no raw pointers cross.
- **Trust boundary 2 — local API:** token required even on loopback; origin checks on WebSocket
  upgrades; JSON schema validation; per-token rate limiting.
- **Trust boundary 3 — remote API:** TLS only, pairing-code bootstrap, device tokens with scopes
  (`read`, `control`, `add`, `admin`), revocation, expiry, brute-force lockout, audit log.
- **Automation:** built-in actions (move/rename/copy/tag/notify/webhook) run unprivileged; shell
  and AppleScript actions require an explicit per-rule consent flag that stores a hash of the exact
  command — changing the command revokes consent. Variables are passed as arguments, never
  interpolated into a shell string.
- **Downloaded files** get the `com.apple.quarantine` attribute; archives are listed, and any
  extraction rejects absolute paths, `..`, and symlinks pointing outside the target.
- **Logs** pass through a redaction layer (Authorization/Cookie/Proxy-Authorization headers, URL
  userinfo, query parameters named like tokens, pairing codes).

## 8. Performance considerations

- Engine on Tokio worker threads; UI never blocks; disk writes use a per-task buffered writer with
  `pwrite` at absolute offsets and periodic `fdatasync`-style flushes tied to the commit watermark.
- Hierarchical token-bucket rate limiter (global → queue → task) with burst control.
- Adaptive segmentation: start with a probe, grow connections while marginal throughput rises,
  shrink on 429/503/RST/stalls; per-host connection caps; slow-segment work-stealing near the end.
- Progress coalescing (≤4 Hz per batch) and row-level diffing keep 1 000+ tasks smooth; the table is
  lazy (`NSTableView`-backed) and reads from an `@Observable` store keyed by task id.
- SQLite writes are batched on a dedicated thread; reads for the UI are served from the in-memory
  task registry, not the DB.

## 9. Testing strategy

- **Rust unit tests** per crate (state machine, URL/filename, segmentation, backoff, rate limiter,
  rules, scheduler, checksum, M3U8/Metalink parsing, redaction, safety).
- **Rust integration tests** with an in-process axum test server that supports Range, throttling,
  redirects, auth, forced disconnects, wrong checksums; recovery tests kill and re-open the store.
- **Server tests** for auth, pairing, scopes, rate limiting, WebSocket events.
- **Swift tests** (`swift test`) for view models and formatting.
- **E2E script** (`scripts/e2e.sh`): build, launch the bundle, add a download through the CLI,
  interrupt, relaunch, verify completion and checksum.

## 10. Repository layout

```
Cargo.toml                    workspace
crates/osprey-domain          ids, task/state machine, queues, rules, schedules, events, errors, engine traits
crates/osprey-runtime         rate limiter, event bus, safe paths, redaction, disk writer, backoff, net config
crates/osprey-store           SQLite persistence, migrations, recovery
crates/osprey-engine-http     HTTP/HTTPS segmented engine, probing, adaptive concurrency, mirrors, metalink, checksum
crates/osprey-engine-ftp      FTP/FTPS
crates/osprey-engine-torrent  librqbit adapter
crates/osprey-media           HLS/M3U8 + direct-media detection
crates/osprey-grabber         site grabber / crawler
crates/osprey-services        TaskManager, queues, scheduler, bandwidth, rules, automation, history, disk, diagnostics, updater → `Engine` facade
crates/osprey-server          axum REST + WS, auth, pairing, embedded web UI
crates/osprey-cli             `osprey` binary: CLI, `server`, `native-host`
crates/osprey-ffi             UniFFI surface for Swift/C#
apps/macos                    SwiftPM package → Osprey.app
extensions/browser            WebExtension (Chrome/Edge/Firefox)
web/remote                    Preact remote UI (embedded in server)
docs/                         architecture, engine, API, CLI, security, threat model, …
scripts/                      build, bundle, dmg, e2e, release
```
