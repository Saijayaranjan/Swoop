# Contributing to Osprey

## Toolchain

```bash
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"   # cargo/rustc (rustup shims)
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

The macOS app is built with SwiftPM (no Xcode required): `scripts/build-macos.sh`.

## Engineering rules (enforced in review)

- No `todo!()`, `unimplemented!()`, placeholder buttons, fake data or dead screens in production paths.
- No `unwrap()`/`expect()` on fallible I/O or parsing outside tests; classify errors with
  `osprey_domain::TaskError` / `DomainError`.
- Every string that came from a network peer, a file name, a torrent, a playlist, a page or a
  remote client goes through `osprey_runtime::safety` before touching the filesystem and through
  `osprey_runtime::redact` before being logged.
- Engines implement `osprey_domain::engine::Transfer`; they never touch SQLite or the UI.
- All user-visible strings in the UI go through the localisation table (`apps/macos/.../Localizable`);
  Rust returns stable keys (`error.dns`, `health.throttled`) and English fallbacks.
- Events, not polling: state changes go through `EventBus::publish`, progress through
  `EventBus::progress` (coalesced).
- Keep modules small; one responsibility per file; document the *why* on non-obvious code.
- Tests: unit tests next to the code; integration tests in `crates/<crate>/tests/` using local
  servers (`axum` test server, in-process FTP, two librqbit sessions) — never the public internet.

## Crate ownership map

| Crate | Responsibility |
|---|---|
| `osprey-domain` | types, state machine, errors, events, engine trait — **stable contract** |
| `osprey-runtime` | rate limiter, event bus, safety, redaction, disk writer, checksum |
| `osprey-store` | SQLite persistence, migrations, recovery |
| `osprey-engine-http` | HTTP/HTTPS segmented engine, adaptive concurrency, mirrors, metalink |
| `osprey-engine-ftp` | FTP/FTPS |
| `osprey-engine-torrent` | BitTorrent (librqbit) |
| `osprey-media` | HLS/M3U8, direct media detection |
| `osprey-grabber` | site crawler |
| `osprey-services` | task manager, queues, scheduler, bandwidth, rules, automation, history, diagnostics, `EngineApi` |
| `osprey-server` | REST + WebSocket, auth, pairing, embedded remote web UI |
| `osprey-cli` | `osprey` binary: CLI, headless server, native-messaging host |
| `osprey-ffi` | UniFFI bindings for Swift (and later C#) |
| `apps/macos` | SwiftUI/AppKit application |
| `extensions/browser` | WebExtension |
| `web/remote` | remote web UI |
