# Security model (what the code guarantees)

1. **Local-first, no accounts, no telemetry.** Nothing is sent anywhere except to the sources the
   user asked to download and, optionally, to devices the user paired.
2. **Secrets live in the OS keychain** (`keyring` crate; macOS Keychain, Windows Credential
   Manager, Secret Service on Linux). SQLite holds only names/usernames and token *hashes*.
3. **Every filesystem write is fenced** by `swoop_runtime::safety` and the writer's
   `expected_root` check; final files are quarantined on macOS.
4. **Every log line is redacted** (`swoop_runtime::redact`).
5. **Three listeners, three trust levels**: Unix socket (same uid + local token), loopback TCP
   (local token; the app binds `settings.remote.local_port`, 41779 by default, `0` disables it),
   remote listener (paired devices with scopes, off by default; TLS unless `--no-tls`).
6. **Consent for code execution is out-of-band**: stored as `(automation_id, action_index, hash)`
   records written only by the desktop UI; the automation engine recomputes the hash of the
   resolved action and refuses on mismatch. Remote clients cannot create exec actions.
7. **Updates are signed, and nothing unverified is ever installed.**
   - *Source.* GitHub Releases of `Saijayaranjan/Swoop` (one constant,
     `swoop_update::GITHUB_REPOSITORY`), queried with `User-Agent: Swoop/<version>` and a cached
     ETag. Drafts are never offered; prereleases only with "Include beta releases". The tag
     (`vX.Y.Z`) must be strictly newer than the running version. The feed URL can be overridden
     (`SWOOP_UPDATE_FEED_URL`) only in debug builds; release builds always ask GitHub.
   - *Trust root.* An Ed25519 public key compiled into the binary
     (`swoop_update::DEFAULT_PUBLIC_KEY_HEX`, overridable only at build time with
     `SWOOP_UPDATE_PUBLIC_KEY`; never at run time). The private key lives with the release
     manager at `~/.config/swoop/update-signing-key` (0600), never in the repository.
   - *Signature.* `Swoop-X.Y.Z.dmg.sig` is a base64 Ed25519 signature over the SHA-256 of
     `Swoop-X.Y.Z.dmg`. The DMG is downloaded over HTTPS only (every redirect hop must also be
     HTTPS, e.g. to `objects.githubusercontent.com`), hashed while it streams, and kept under a
     temporary name until the signature verifies; a mismatch deletes it.
   - *Install (macOS app only).* The signature is verified again right before
     `hdiutil attach -nobrowse -readonly -noautoopen`. The image must hold exactly one
     `Swoop.app` whose `CFBundleIdentifier` is `app.swoop.desktop`, whose version is strictly
     newer than the running one and equal to the release's, and which passes
     `codesign --verify --deep --strict`. It is copied with `ditto` into `.Swoop-update/` next to
     the running bundle (and checked again there). After the app quits cleanly (downloads paused
     and persisted), the bundled `swoop update apply` helper waits for its PID to exit, swaps the
     bundles with two renames, strips `com.apple.quarantine`, relaunches and confirms the new
     process is running, and otherwise restores the previous bundle. Any failure refuses the
     update and cleans up. If the app's folder isn't writable (or the app is translocated), the
     update is refused with a message to move Swoop to /Applications.
   - *Headless.* The CLI (`swoop update check`) and `swoop server` only report or notify about
     updates; they never install one. `POST /api/v1/updates/download` only downloads and verifies.
   - Losing the private key means existing installs can't verify any future update: back it up.
8. **Panics never cross the FFI**; hostile inputs produce classified errors, not crashes.
9. **Dependencies**: TLS is rustls everywhere (no OpenSSL). A generated licence/advisory report
   (`cargo deny`) is not yet part of the repository.
