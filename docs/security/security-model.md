# Security model (what the code guarantees)

1. **Local-first, no accounts, no telemetry.** Nothing is sent anywhere except to the sources the
   user asked to download and, optionally, to devices the user paired.
2. **Secrets live in the OS keychain** (`keyring` crate; macOS Keychain, Windows Credential
   Manager, Secret Service on Linux). SQLite holds only names/usernames and token *hashes*.
3. **Every filesystem write is fenced** by `osprey_runtime::safety` and the writer's
   `expected_root` check; final files are quarantined on macOS.
4. **Every log line is redacted** (`osprey_runtime::redact`).
5. **Three listeners, three trust levels**: Unix socket (same uid + local token), loopback TCP
   (local token; the app binds `settings.remote.local_port`, 41779 by default, `0` disables it),
   remote listener (paired devices with scopes, off by default; TLS unless `--no-tls`).
6. **Consent for code execution is out-of-band**: stored as `(automation_id, action_index, hash)`
   records written only by the desktop UI; the automation engine recomputes the hash of the
   resolved action and refuses on mismatch. Remote clients cannot create exec actions.
7. **Updates are signed**: the Ed25519 public key comes from `OSPREY_UPDATE_PUBLIC_KEY` (at build
   time, or at run time); without it update checks are refused. The signature over the declared
   SHA-256 is checked before download, the archive is hashed while downloading and only renamed
   to its final name when hash and size match; downgrades are refused. The engine never extracts
   or installs anything itself.
8. **Panics never cross the FFI**; hostile inputs produce classified errors, not crashes.
9. **Dependencies**: TLS is rustls everywhere (no OpenSSL). A generated licence/advisory report
   (`cargo deny`) is not yet part of the repository.
