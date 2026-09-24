# Osprey status

A factual inventory of what the code does as of this revision, taken from reading the sources
rather than the design documents. "Complete" means the feature works end to end through the
engine and at least one front end (app, CLI, REST, or extension) and has no known functional
gap. "Partial" lists what is missing.

## Verification at this revision

| Check | Result |
|---|---|
| `cargo build --workspace` | passes |
| `cargo clippy --workspace --all-targets -- -D warnings` | passes, no warnings |
| `cargo test --workspace` | 253 passed, 0 failed, 0 ignored (48 test binaries and doc-test runs) |
| `extensions/browser`: `npm run lint` / `npm run build` / `npm test` | tsc clean; Chrome, Firefox and Edge bundles built; 47 of 47 tests pass |
| `web/remote`: `npm run lint` / `npm run build` / `npm test` | tsc clean; bundle 29.7 KB gzip (budget 150 KB); 53 of 53 tests pass |
| macOS app (`swift build`), Docker image | not built as part of this review |

`cargo fmt --all -- --check` reports formatting drift in files that predate this review, mostly
in `osprey-cli` and `osprey-server`. It is not enforced in CI.

## Feature inventory

| Feature | Status | Notes |
|---|---|---|
| HTTP/HTTPS | **Complete** | Probe with retry, a segmented download into a part file, resume from a checkpoint after a restart, and validation of `ETag`, `Last-Modified` and size on resume. If the server ignores `Range`, it degrades to one connection and truncates the part file first. Redirects are capped, and a public origin can't redirect to a local or private address. Filenames come from `Content-Disposition` and are sanitised. The file is flushed and renamed from its part file only after every segment is complete. Truncated bodies are detected when the size is known. It detects when a server sends HTML in place of the file. Mirrors are supported, as are per-host and global connection caps. |
| FTP/FTPS | **Partial** | Osprey has its own Tokio client with USER/PASS, EPSV/PASV, REST resume, explicit TLS (`AUTH TLS`, PBSZ, PROT), MLSD/LIST and rate limiting. Missing: active mode, proxies for FTP, and segmented (multi-connection) FTP. The `suppaftp` workspace dependency is declared but not used. |
| BitTorrent/magnet | **Partial** | Built on librqbit 9. It handles .torrent files and magnets (metadata resolution), file selection, seeding ratio and time limits, a tracker registry with HTTP/UDP probes, DHT, a peer list, a piece map and the BEP-27 private-torrent policy. Missing: PEX control, protocol encryption (MSE/PE), real per-file priorities and a sequential toggle (librqbit doesn't expose these), uTP listening, WebTorrent trackers, and swarm availability or seed counts. |
| Metalink | **Partial** | Metalink v3/v4 parsing, mirrors, and the best available whole-file hash verified before promotion. Missing: multi-file metalinks download only the first file (a warning is logged), and `<pieces>` hashes and `<metaurl>` torrents are ignored. |
| HLS (non-DRM) | **Partial** | Handles master/media playlists, variant choice, byte ranges, `EXT-X-MAP`, AES-128 clear-key decryption, per-segment checkpoints, merge, and an optional ffmpeg remux to MP4. `SAMPLE-AES`/FairPlay/Widevine and live playlists (no `EXT-X-ENDLIST`) are refused. Missing: separate alternate-audio renditions (`EXT-X-MEDIA TYPE=AUDIO`) are parsed but not downloaded or muxed. DASH is detected but not downloaded. |
| Segmented adaptive acceleration | **Complete** | Work-stealing segment splits, growth experiments kept only on a 15% or better gain, immediate shrink on 429/503/`Retry-After` or reset bursts, per-client throttling detection and an endgame split of slow tails. It can be switched off per task or globally. |
| Queues | **Complete** | CRUD, ordering, per-queue concurrency, queue directory, queue pause/resume, and admission by priority then position. Global caps apply to active downloads and active torrents; seeding doesn't take a slot. |
| Scheduler | **Complete** | Schedule windows and conditions with hysteresis gate tasks and queues. Actions run when a window opens or closes. An explicit start overrides the gate for that task. |
| Bandwidth control | **Complete** | A limiter tree (global, then queue, then task) for download and upload, live changes to running tasks, traffic modes (unlimited, balanced, browsing, custom) and an "Optimize" heuristic. |
| Rules | **Complete** | Deterministic add-time rules (category, directory, queue, options, tags) plus completion actions: move, Finder tags, reveal, and run automation. There's a dry-run endpoint. |
| Automation | **Complete** | Actions: move, copy, rename, notify, webhook, command, shell, sandboxed script, emit event, tag, and platform actions (open, reveal, Finder tag, AppleScript). Code-running actions need a consent hash that only the desktop UI can write. Editing an action invalidates its consent. Details are under the security model. |
| Site grabber | **Complete** | A bounded, robots-aware crawler (streaming `lol_html`, scope, depth, page and concurrency limits, per-host politeness). Files are classified by extension or MIME and can be probed with HEAD. Discovered files can be added as tasks. |
| Browser extensions + native bridge | **Partial** | An MV3 extension for Chrome, Edge and Firefox: download interception with size, extension and domain rules, context menus, a popup with live rows, media detection, and bulk link collection. The native host relays to the local socket. Missing: store publication. Chromium users must paste the extension ID when installing the host manifest. No Safari build. |
| Remote control | **Complete** | REST API plus WebSocket events, an embedded Preact web UI, pairing with one-time codes, scoped device tokens, TLS with a persisted self-signed certificate and its fingerprint, rate limiting, lockout and an audit log. |
| Headless server | **Complete** | `osprey server` (or `osprey --headless`) runs the full engine with a Unix socket, optional loopback TCP and an optional remote listener. Credentials fall back to a mode-0600 file when no keychain is available. |
| CLI | **Complete** | add, list, status, pause, resume, retry, restart, cancel, remove, pause-all, resume-all, watch, queue, limit, mode, history, export, import, pair, devices, diagnostics, server and native-host. Exit codes are documented in `docs/cli.md`. |
| Docker | **Partial** | A multi-stage `Dockerfile` (web UI, release binary, slim Debian with a non-root user) and `docker/compose.yaml` with a healthcheck. Not built or run as part of this review (no Docker on the review machine). There's no published image. |
| Diagnostics | **Complete** | Per-task log ring, a health score, a "copy diagnostics" text report (REST `diagnostics.txt`, CLI `diagnostics`), a redacted process log ring and daily rolling log files. |
| Crash recovery | **Complete** | SQLite WAL with one writer. Checkpoints are written only after a barrier flush. At startup, a recovery table reconciles the persisted state with what's on disk: re-queue with or without the checkpoint, re-verify, re-process, adopt a finished file, or re-add torrents. The services layer then re-queues anything left active. |
| Security | **Complete** (see the model below) | Recently hardened: remote save-path confinement, the native-host allowlist, safe remove-with-files and staged update downloads. |
| Accessibility | **Partial** | SwiftUI views carry about 22 `accessibilityLabel`s and a few values and elements. The web UI has 28 ARIA attributes. There's no VoiceOver or keyboard-only audit and no contrast audit. |
| i18n | **Partial** | English, Hindi and Tamil. The extension is fully translated (66 of 66 keys). The web UI has 44 of 256 keys translated in hi/ta, with fallback to English. The macOS `Localizable.strings` has about 109 of 208 lines in hi/ta. The Rust side returns stable keys plus English fallbacks. |
| Updates | **Partial** | The Ed25519 signature over the SHA-256 is verified before download. The archive is hashed and its size checked, and it's staged under a temporary name. Downgrades are refused, and https is required. Missing: the verified DMG isn't installed automatically (it's handed to the app or user). The signing key has to be supplied through `OSPREY_UPDATE_PUBLIC_KEY`; without it, update checks return "unavailable". There's no published feed yet. |
| Import/export | **Complete** | A JSON bundle containing settings, queues, categories, rules, schedules, automations and recipes, plus tasks and history optionally. IDs are regenerated on conflict unless overwrite is set. Automations are imported without consent. Available through REST, the CLI and the app. |
| Plugins | **Partial** | Manifest discovery, enable/disable, a permission grant model, and REST and app surfaces. Plugin code execution (an out-of-process host) isn't implemented. |
| Archive extraction | **Partial** | Listing and selective extraction of ZIP, TAR, TAR.GZ/TGZ and GZ, with no zip-slip, no symlinks and no absolute paths. Local callers only. RAR and 7z are only recognised by their magic bytes; they aren't extracted. |

## Architecture

```
            ┌──────────── apps/macos (SwiftUI/AppKit, OspreyKit) ────────────┐
            │  UniFFI Swift bindings  ──►  libosprey_ffi.a (osprey-ffi)       │
            └───────────────────────────────┬────────────────────────────────┘
                                            │ in-process
 osprey CLI ──unix socket / https──►  osprey-server (REST + WS + web UI)
 (osprey-cli)                               │
 browser ext ─► native host (osprey native-host) ─► unix socket
                                            ▼
                              osprey-services  (EngineApi: the one command surface)
       ┌─────────────┬──────────────┬───────┴──────┬──────────────┬──────────────┐
  engine-http   engine-ftp   engine-torrent    media (HLS)    grabber   automation/archive/
  (+metalink)                 (librqbit)                                update/plugins
       └──────────── osprey-runtime (net, safety, disk writer, limiter, bus, redact) ──┘
                              osprey-store (SQLite, migrations, recovery)
                              osprey-domain (types, state machine, errors, events)
```

- **osprey-domain**: I/O-free types: tasks, the state machine, errors, events, settings, rules and schedules.
- **osprey-runtime**: HTTP client factory, redirect and SSRF policy, filesystem safety, the file writer, rate limiters, the event bus, checksums and redaction.
- **osprey-store**: SQLite in WAL mode with one writer thread, forward-only migrations (`user_version`) and startup recovery.
- **Engines**: `osprey-engine-http` (plus Metalink and mirrors), `osprey-engine-ftp`, `osprey-engine-torrent` (librqbit) and `osprey-media` (HLS). Each implements the runtime `Transfer` trait.
- **osprey-grabber**, **osprey-automation**, **osprey-archive**, **osprey-update** and **osprey-plugins**: feature crates.
- **osprey-services**: `Engine`, which implements `EngineApi`. It handles admission, lifecycle, rules, the scheduler, bandwidth, devices and pairing, history, and import/export.
- **osprey-server**: axum REST, WebSocket and the embedded web UI on three listener kinds. They are the Unix socket (0600, same uid), loopback TCP, and the remote listener (TLS by default).
- **osprey-ffi**: a UniFFI object (`OspreyEngine`) that owns a Tokio runtime, the services engine and the local API server. The macOS app links it as a static library. The remote listener starts when the settings enable it.
- **osprey-cli**: a single `osprey` binary that serves as the CLI client, the headless server (it runs services and the server in-process) and the browser native-messaging host. The app bundle ships it in `Contents/Helpers/osprey`.
- **osprey-testserver**: a local HTTP fixture server used by the tests.
- **extensions/browser**: an MV3 extension (TypeScript, esbuild) that talks only to the native host.
- **web/remote**: the Preact remote UI, embedded into `osprey-server` through `rust-embed` at compile time.

## Core technologies

| Component | Version (declared, then resolved in `Cargo.lock` or `node_modules`) |
|---|---|
| Rust | edition 2021, `rust-version` 1.85, stable toolchain (reviewed with rustc 1.98.1) |
| tokio | 1.47, resolved 1.53.1 |
| reqwest (rustls, http2, socks) | 0.13, resolved 0.13.5; rustls 0.23.45 |
| axum / axum-server (tls-rustls) / hyper | 0.8 (0.8.9) / 0.7 (0.7.3) / 1 (1.11.1) |
| rusqlite (bundled SQLite) | 0.32 (0.32.1) |
| librqbit | 9 (9.0.1) |
| uniffi | 0.29 (0.29.5) |
| ed25519-dalek | 2 (2.2.0) |
| lol_html / quick-xml | 2 (2.9.0) / 0.37 (0.37.5) |
| keyring | 3 (3.6.3) |
| clap | 4.5 (4.6.7) |
| rcgen | 0.13 (0.13.2) |
| Swift package | swift-tools-version 5.9, macOS 14+, SwiftUI/AppKit plus Charts, Quartz, UserNotifications and ServiceManagement; built with Command Line Tools only (reviewed with Swift 6.4) |
| Extension | TypeScript ^5.6, esbuild ^0.24 (0.24.2), webextension-polyfill ^0.12 |
| Web UI | Preact ^10.24 (10.29.8), TypeScript ^5.7, esbuild ^0.24; Node ≥ 23.6 for `npm test` (native TS). The Docker build stage uses Node 22, which is enough for `npm run build`. |

## Security model

What the documentation claims (`docs/security/`) and what the code actually enforces:

- **Listeners.**
  - The Unix socket is mode 0600, bound through a private staging directory, and drops peers with a different uid. The local token is accepted there and on loopback TCP only.
  - The loopback TCP listener refuses non-loopback addresses. The app binds it on `settings.remote.local_port` (41779 by default; `0` disables it).
  - The remote listener is off by default. It uses TLS with a persisted self-signed certificate unless `--no-tls` is given, in which case a warning is logged. The local token is always rejected on the remote listener.
- **Authorization.** One policy table (`osprey-server/src/auth.rs`) maps every route to a scope (read, add, control or admin); unknown routes need admin. Archives are local-only. Device tokens are 32 random bytes and only their SHA-256 hash is stored. They expire and can be revoked. Failed attempts are locked out per IP. Remote requests are audited.
- **Pairing.** Codes are 8 characters from a 32-letter alphabet, valid for 120 s, single use and compared in constant time. The pair route is public but rate limited and subject to lockout.
- **CORS and origin.**
  - Requests that carry an `Origin` header must match the host, a configured allow-list entry, or a browser-extension scheme. A bearer token is always required, and query tokens are accepted only for `/events` and `/tasks/{id}/file`.
  - There's no cookie authentication, so CSRF and DNS rebinding can't act without the token.
- **Filesystem.**
  - Every server-, torrent-, playlist- or archive-supplied name is sanitised. Relative paths are checked for traversal and depth.
  - Destination directories are checked against a deny-list: system prefixes, `~/.ssh`, LaunchAgents, `~/.config` and others.
  - The file writer checks `expected_root` and opens leaf files with `O_NOFOLLOW`.
  - Completed files get the quarantine attribute.
  - Remote devices may only save inside the download folder or configured queue folders.
  - "Remove with files" deletes only a completed task's promoted file, strictly inside its folder.
- **Network.** A public origin can't redirect to a loopback, private or link-local target. Webhooks resolve DNS and refuse private targets, and plain http is allowed only to localhost. Header names and values are validated, and hop-by-hop and `Range` headers can't be overridden. TLS 1.2 is the minimum. Certificate exceptions are per host and opt-in.
- **Native messaging.** The browser pins the host manifest to the extension ID. The host relays only `/api/v1/tasks…` and `/api/v1/media…`, with a fixed set of methods, and rejects ambiguous paths. Messages are capped at 1 MiB.
- **Automation.** Command, shell and AppleScript actions need a consent record whose BLAKE3 hash matches the resolved action. Only the desktop UI writes consent records (there's no REST route for it). Remote callers can't create or link code-running automations, and imports never carry consent. Commands run with an absolute program path and argv; variables are passed as environment variables. The sandboxed script language has no I/O and is capped by length, token count, steps and time.
- **Updates.** The Ed25519 signature over the declared SHA-256 is checked before download. The archive is hashed while it downloads and staged under a temporary name. Downgrades are refused, and https is required. The key comes from `OSPREY_UPDATE_PUBLIC_KEY`.
- **Secrets.** Credentials live in the OS keychain, with a 0600 file fallback on headless Linux. Log lines are redacted.
- **Differences from the older docs, now corrected in `docs/security/`.**
  - Loopback TCP is on by default in the app.
  - On cross-origin redirects, only reqwest's sensitive headers (`Authorization`, `Cookie`, `Proxy-Authorization`) are dropped, not all custom headers.
  - The `docs/security/dependencies.md` licence report doesn't exist.
  - The update key isn't compiled in unless it's set at build time.

## Known limitations

- **Metalink:** multi-file metalinks download only the first file.
- **HLS:** HLS has no DASH support and no separate alternate-audio renditions. Remuxing to MP4 needs ffmpeg on the system.
- **HTTP:** if the size is unknown (chunked with no length), the download uses one connection with no resume. A body cut off by a close-delimited HTTP/1.0 response can't be detected.
- **FTP:** passive mode only, no FTP proxy, one connection.
- **Torrents:** see the BitTorrent row above (no PEX or encryption switches, no per-file priority, no uTP listen).
- **Plugins:** manifest and permission model only; no code execution.
- **Archives:** RAR and 7z aren't extracted.
- **Updates:** there's no automatic installer, no published feed and no bundled key.
- **Platforms:** the desktop app is macOS 14+ only. Linux and Docker run only the headless server and CLI. There's no Windows front end.
- **Translations:** the web and macOS translations for Hindi and Tamil are partial.
- **Tests:** the macOS app has only a small Swift test target (9 tests).
- **Recovery:** a task left in `Verifying` or `Processing` by an older build (before `file_path` was persisted at that step) can still be matched against `<directory>/<name>` by size during recovery.
- **Admin scope:** a remote device with the `admin` scope can change `download_directory` in settings, and so can widen where it may save. Admin is intentionally all-powerful.

## Running locally

```bash
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
cargo build --workspace
cargo test --workspace

# Headless engine + API (Unix socket in the data dir; add --remote 0.0.0.0:41780 for LAN)
cargo run -p osprey-cli -- server --download-dir ~/Downloads
# In another terminal
cargo run -p osprey-cli -- add https://example.com/file.iso
cargo run -p osprey-cli -- list
cargo run -p osprey-cli -- pair        # prints a pairing code for a remote device

# Web UI (embedded at build time; build it before building osprey-server/osprey-cli)
(cd web/remote && npm install && npm run build)

# Browser extension (load dist/chrome or dist/firefox unpacked)
(cd extensions/browser && npm install && npm run build)
osprey native-host --install-manifest chrome --extension-id <32-letter id>

# macOS app (Rust static lib → UniFFI bindings → SwiftPM → Osprey.app, ad-hoc signed)
scripts/build-macos.sh --debug        # or --release [--universal] [--dmg]
open build/Osprey.app
```

The app can also install the native-messaging manifests itself from Settings. It writes a small
launcher script that execs `Osprey.app/Contents/Helpers/osprey native-host`.

Docker: `docker compose -f docker/compose.yaml up -d --build`. This exposes the TLS remote
listener on 41780 and stores data and downloads in named volumes.

## Creating a release

`scripts/build-macos.sh --release [--universal] [--dmg]` does the following (another agent was editing these scripts during this review, so check the current version):

1. It builds `osprey-ffi` and `osprey-cli` in release mode for `aarch64-apple-darwin`. With `--universal` it also builds `x86_64-apple-darwin` when that target is installed, and the outputs are `lipo`'d.
2. It generates the UniFFI Swift bindings, header and modulemap with the bundled `uniffi-bindgen`.
3. It runs `swift build -c release --product OspreyApp`.
4. It assembles `build/Osprey.app`: the binary, `Contents/Helpers/osprey`, resources and localisations. `Info.plist` gets the version from `Cargo.toml` and a timestamp build number.
5. It code-signs the helper and then the app with the hardened runtime and `Osprey.entitlements`, using `OSPREY_SIGN_IDENTITY` if it's set. Without it the signature is ad hoc (`-`). It then runs `codesign --verify --deep --strict`.
6. With `--dmg`, `scripts/make-dmg.sh` stages the app plus an `/Applications` symlink and creates a UDZO DMG at `build/Osprey-<version>.dmg`.

Signing and notarisation caveats (there's no Developer ID):

- An ad-hoc signature isn't trusted by Gatekeeper. A DMG downloaded from the internet is quarantined and blocked ("cannot be opened because the developer cannot be verified"). Users must right-click > Open, or run `xattr -dr com.apple.quarantine /Applications/Osprey.app`.
- Notarisation (`notarytool`) and stapling need an Apple Developer ID Application certificate. With one: set `OSPREY_SIGN_IDENTITY`, then run `xcrun notarytool submit build/Osprey-<v>.dmg --wait` and `xcrun stapler staple`. None of this is scripted yet.
- An ad-hoc identity changes with every build. Keychain items and privacy grants (for example the automation/AppleScript prompts) may be requested again after an update.
- For in-app updates, generate an Ed25519 key pair and build with `OSPREY_UPDATE_PUBLIC_KEY=<hex public key>`. Then sign `sha256(dmg)` with the private key and publish an `appcast.json` in the format documented in `crates/osprey-update/src/lib.rs`. Without the key, update checks are disabled.
- Chromium users need the extension's store ID in the native-host manifest. Firefox uses the fixed gecko ID `osprey@osprey.app`.

## Suggested future work

1. Developer ID signing, notarisation and stapling in the release script, plus a published, signed update feed and an installer step for verified updates.
2. Expanding multi-file metalinks into one task per file, and separate HLS audio renditions, muxed with ffmpeg.
3. A DASH (non-DRM) downloader.
4. An out-of-process plugin runtime on top of the existing permission model.
5. Full Hindi and Tamil translations for the web UI and the app, and an accessibility audit (VoiceOver, keyboard navigation, contrast).
6. A Swift UI test target covering the main flows, and CI that runs `cargo fmt --check`, clippy, the tests and both npm builds.
7. FTP active mode and FTP-through-proxy.
8. RAR and 7z extraction through an external tool with the same safety checks.
9. A `cargo deny` licence and advisory report checked into `docs/security/`.
10. Recovery: requiring a recorded `file_path` before adopting a finished file, once older databases are no longer a concern.
