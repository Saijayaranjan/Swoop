# Security model (what the code guarantees)

1. **Local-first, no accounts, no telemetry.** Nothing is sent anywhere except to the sources the
   user asked to download and, optionally, to devices the user paired.
2. **Secrets live in the OS keychain** (`keyring` crate; macOS Keychain, Windows Credential
   Manager, Secret Service on Linux). SQLite holds only names/usernames and token *hashes*.
3. **Every filesystem write is fenced** by `osprey_runtime::safety` and the writer's
   `expected_root` check; final files are quarantined on macOS.
4. **Every log line is redacted** (`osprey_runtime::redact`).
5. **Three listeners, three trust levels**: Unix socket (same uid + local token), loopback TCP
   (local token, off by default on macOS), remote TLS (paired devices with scopes, off by default).
6. **Consent for code execution is out-of-band**: stored as `(automation_id, action_index, hash)`
   records written only by the desktop UI; the automation engine recomputes the hash of the
   resolved action and refuses on mismatch. Remote clients cannot create exec actions.
7. **Updates are signed**: the Ed25519 public key is compiled into the app; unsigned or
   downgraded archives are never extracted.
8. **Panics never cross the FFI**; hostile inputs produce classified errors, not crashes.
9. **Dependencies are audited** (`cargo deny`-compatible list in `docs/security/dependencies.md`,
   generated at release time): all Rust dependencies are MIT/Apache-2.0/BSD/ISC/MPL-2.0/CC0
   licensed; no OpenSSL.
