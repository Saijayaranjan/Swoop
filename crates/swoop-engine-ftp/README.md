# swoop-engine-ftp

Tokio-native FTP/FTPS client and `Transfer` implementation.

- Schemes: `ftp://` (plain), `ftps://` (explicit `AUTH TLS`; implicit TLS when the port is 990), `ftpes://`.
- Commands: USER/PASS, FEAT, TYPE I, EPSV (PASV fallback with NAT workaround), SIZE, MDTM, REST, RETR, MLSD/LIST, ABOR, QUIT, PBSZ/PROT for FTPS data channels.
- Resume with `REST` from the committed watermark; servers without REST restart from zero.
- Reconnects with backoff on data-connection loss, timeouts and 4xx transient replies; resumes from the last flushed byte.
- Pause protocol: stops within ~1 s, flushes, checkpoints, returns `Paused`.
- Rate limiting through the task's hierarchical limiter; single connection (FTP has no parallel ranges).
- TLS verification via the platform verifier; per-host exceptions from settings disable it (with a warning in the UI).
- Credentials: URL userinfo → task credential (`TransferSecrets`) → anonymous. Passwords never appear in logs.
- Limitations: no proxy support for FTP (documented), no active mode, no upload.
