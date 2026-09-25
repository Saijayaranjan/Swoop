# Threat model

## Assets
- The user's files and filesystem (integrity, no writes outside chosen destinations).
- Credentials: site logins, proxy passwords, API tokens, pairing codes, tracker passkeys.
- The user's privacy: URLs, filenames, browsing context never leave the machine.
- The machine's integrity: automation must never execute code the user did not approve.
- Availability of the engine (no crash/exhaustion from hostile inputs).

## Trust boundaries and untrusted inputs

| Boundary | Untrusted input | Controls |
|---|---|---|
| Network → engine | HTTP headers (`Content-Disposition`, `Content-Type`, `Location`, `Retry-After`), bodies, FTP replies, torrent metadata (paths, sizes, trackers), playlists (segment URLs, key URIs), Metalink XML, HTML pages | `swoop_runtime::safety` (sanitise names, reject traversal, depth/length caps), `filename` (RFC 6266/5987 parsing keeps only the last path component), redirect policy refuses public→local hops, `net::assert_public_target` for server-supplied URLs, size-bounded parsers (quick-xml limits, playlist line caps, `lol_html` streaming), tracker/peer counts bounded |
| Extension → native host → local API | JSON messages | host relays only `/api/v1/tasks…` and `/api/v1/media…` (allowlist) with GET/POST/PUT/PATCH/DELETE, rejects `..`, `//`, `%`, `#`, backslashes and whitespace in paths; token never leaves the host; 1 MiB message cap |
| Remote device → server | JSON bodies, query params, WebSocket frames | TLS only, bearer tokens hashed at rest, scopes, per-token rate limits, lockout, JSON schema validation via serde (deny unknown fields on settings), body size limits (1 MiB; torrent upload 10 MiB), origin allow-list, audit log; exec automation actions refused; consent never grantable remotely |
| UI/API → filesystem | destination directories, rename templates, extract targets | `validate_destination_dir` prefix/prefix-of-home deny list, `ensure_within`, `O_NOFOLLOW` leaf opens, canonical-root check for part files, zip-slip protection on extraction, quarantine attribute on completed files |
| Import file → engine | ExportBundle JSON | same validators as the API; ids regenerated on conflict unless `overwrite`; exec actions imported **without** consent |
| Update feed → app | appcast JSON, archive | Ed25519 signature over the declared SHA-256 verified before download; the archive is written under a temporary name and only renamed into place after its SHA-256 and size match; version must be greater; no key configured → updates unavailable |
| Plugins | manifest + code | disabled by default; explicit permission grant; run in a separate process with a restricted API (this version: manifests + permission model + lifecycle; execution host documented as future) |

## Specific attacks considered

- **Path traversal / zip-slip** via `Content-Disposition`, torrent file lists, archive entries → `sanitize_relative_path`, `ensure_within`, extraction refuses absolute paths, `..`, symlinks.
- **Symlink swap (TOCTOU)** → leaf `O_NOFOLLOW`; canonical parent must remain under the destination root at open and before rename.
- **Sensitive destination** (LaunchAgents, `~/.ssh`, system dirs) → denied by `validate_destination_dir`, including for rule `SaveTo` and remote `add`.
- **Remote save paths** → a paired device may only name a directory inside the default download folder or a queue folder configured on this computer (`add`, batch add, recipe apply, grabber add, task edit); anything else is `403`. `GET /tasks/{id}/file` only serves a file inside the task's own folder.
- **Deleting files** → "remove with files" deletes only the file Swoop promoted from its part file (a completed task's recorded `file_path`, strictly inside the task folder); an unfinished task only loses its part data, never a same-named file the user already had.
- **SSRF** through redirects, HLS key URIs, Metalink mirrors, webhooks → local/link-local/metadata targets refused unless the origin was already local; webhook hosts re-resolved.
- **Header injection** → CR/LF rejected; hop-by-hop headers cannot be set; on cross-origin redirects reqwest drops `Authorization`, `Cookie` and `Proxy-Authorization` (other custom headers are forwarded).
- **Credential leakage in logs** → `redact()` on every logged string; secrets in Keychain; token hashes only in SQLite; no URLs in analytics (there are none).
- **Remote code execution via automation** → exec actions require store-side consent hashes granted only through the local UI; commands run with absolute program paths, arguments passed as argv (never shell-interpolated), variables via environment; AppleScript executed by the app layer with the same consent gate; sandboxed script runtime has no I/O.
- **Brute force on pairing** → 8-char codes (~2^41), single use, 2-minute TTL, 5 attempts per IP then 15-minute lockout, constant-time compare.
- **Token theft** → tokens shown once, hashed at rest, revocable per device, expiring; TLS pinning by fingerprint in the QR.
- **DNS rebinding / CSRF against the local API** → token required in a header (query token only for WebSocket/file routes), origin allow-list, the Unix socket additionally checks the peer uid. The app also binds loopback TCP on `settings.remote.local_port` (41779 by default; `0` disables it).
- **Resource exhaustion** → per-host and global connection caps, bounded writer memory per file, bounded event channels, bounded parsers, crawler limits (pages, depth, concurrency, per-host politeness), SQLite single writer with batching.
- **Malicious file execution** → completed files carry the quarantine attribute; Swoop never opens files automatically unless the user enabled `open_when_done` for that task.
- **Private torrent policy** → DHT/PEX/extra trackers never applied to private torrents (BEP-27).
- **DRM / access control** → HLS with `SAMPLE-AES`, `KEYFORMAT` other than identity, or EME streams is refused (`ProtectedContent`); no credential harvesting from browsers beyond cookies for the exact URL the user chose to download.

## Out of scope
- A compromised local user account (the engine runs as the user).
- Physical access to an unlocked machine.
- Malicious browser extensions with `nativeMessaging` for our host id (the host manifest pins the published extension ids; a modified browser can bypass this — mitigated by the host refusing sensitive routes).
