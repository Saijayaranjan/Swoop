# Contributing to Swoop

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
  `swoop_domain::TaskError` / `DomainError`.
- Every string that came from a network peer, a file name, a torrent, a playlist, a page or a
  remote client goes through `swoop_runtime::safety` before touching the filesystem and through
  `swoop_runtime::redact` before being logged.
- Engines implement `swoop_domain::engine::Transfer`; they never touch SQLite or the UI.
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
| `swoop-domain` | types, state machine, errors, events, engine trait — **stable contract** |
| `swoop-runtime` | rate limiter, event bus, safety, redaction, disk writer, checksum |
| `swoop-store` | SQLite persistence, migrations, recovery |
| `swoop-engine-http` | HTTP/HTTPS segmented engine, adaptive concurrency, mirrors, metalink |
| `swoop-engine-ftp` | FTP/FTPS |
| `swoop-engine-torrent` | BitTorrent (librqbit) |
| `swoop-media` | HLS/M3U8, direct media detection |
| `swoop-grabber` | site crawler |
| `swoop-services` | task manager, queues, scheduler, bandwidth, rules, automation, history, diagnostics, `EngineApi` |
| `swoop-server` | REST + WebSocket, auth, pairing, embedded remote web UI |
| `swoop-cli` | `swoop` binary: CLI, headless server, native-messaging host |
| `swoop-ffi` | UniFFI bindings for Swift (and later C#) |
| `apps/macos` | SwiftUI/AppKit application |
| `extensions/browser` | WebExtension |
| `web/remote` | remote web UI |

## Releasing

Installed copies of Swoop update themselves from
[GitHub Releases](https://github.com/Saijayaranjan/Swoop/releases). A release is only offered
when it is published (not a draft), its tag is `vX.Y.Z` (prereleases like `v1.3.0-beta.1` reach
only users who turned on "Include beta releases"), and it carries exactly these two assets:

- `Swoop-X.Y.Z.dmg`: the universal installer DMG from `scripts/build-macos.sh`.
- `Swoop-X.Y.Z.dmg.sig`: one line of base64, the Ed25519 signature over the DMG's SHA-256,
  made with the release signing key.

> [!WARNING]
> **Back up the release signing key.** The private key is
> `~/.config/swoop/update-signing-key` on the release machine (chmod 600, directory 700). It is
> not in this repository and must never be committed or shared. Every installed copy of Swoop
> trusts only its public half (`DEFAULT_PUBLIC_KEY_HEX` in `crates/swoop-update/src/lib.rs`).
> **If the private key is lost, existing installs can never verify another update**; users would
> have to download a new build by hand. Keep an encrypted offline backup (for example in a
> password manager and on an encrypted USB drive) before publishing the first release.

To cut a release:

```bash
scripts/release.sh 0.2.0          # bump, build universal DMG, sign, verify, write notes
$EDITOR build/release-notes-0.2.0.md
git add Cargo.toml Cargo.lock && git commit -m "Release 0.2.0"
git tag -a v0.2.0 -m "Swoop 0.2.0"
git push origin main v0.2.0
scripts/release.sh 0.2.0 --no-build --publish   # or run the printed `gh release create` yourself
```

`scripts/release.sh` sets the `[workspace.package]` version in `Cargo.toml` (the app's
`Info.plist` and the CLI take their version from it), runs
`scripts/build-macos.sh --release --universal --dmg`, signs the DMG with
`swoop update sign` (a hidden subcommand of the bundled CLI), verifies the signature with the key
compiled into the app it just built, writes release notes from
`scripts/release-notes-template.md`, and prints the exact `gh release create` command. It only
publishes when you pass `--publish`, and `--verify-tag` makes `gh` refuse unless the tag is
already pushed.

Other key operations:

- A different signing key location: `SWOOP_UPDATE_SIGNING_KEY=/path/to/key scripts/release.sh …`.
- Rotating the key: `swoop update keygen --out <new path>` prints the new public key. Ship one
  release, signed with the **old** key, whose `DEFAULT_PUBLIC_KEY_HEX` is the **new** public key;
  sign every later release with the new key.
- A build that trusts another key (forks, testing): set `SWOOP_UPDATE_PUBLIC_KEY=<hex>` when
  building. There is no run-time override.
- Testing the updater locally: debug builds (`scripts/build-macos.sh --debug`) honour
  `SWOOP_UPDATE_FEED_URL`, pointing at a release JSON (the `/releases/latest` shape) served from
  loopback, and `SWOOP_UPDATE_TEST_INSTALL=1`, which takes the Install and Relaunch path as soon
  as the update verifies. Release builds ignore both. The helper logs to
  `~/Library/Logs/Swoop/updater.log`.

The GitHub repository must be public: the app queries the REST API without authentication
(60 requests per hour per IP, and a rate-limited check is simply retried later).
