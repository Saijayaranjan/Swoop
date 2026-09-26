# `swoop` command line

`swoop` is one binary with three jobs:

1. **CLI client** — talks to a running engine (the desktop app, or `swoop server`) over its
   local Unix socket, or over HTTP(S) to a paired remote server.
2. **Headless server** (`swoop server` / `swoop --headless`) — runs the engine and the API
   server in one process, for machines without the desktop app (Linux, Docker, NAS).
3. **Browser native-messaging host** (`swoop native-host`) — a stateless relay the browser
   extension launches; not meant to be run by hand.

## Global options

These apply before the subcommand and work with every command that talks to the engine:

| Flag | Meaning |
|---|---|
| `--json` | Print machine-readable JSON instead of a human table. |
| `--data-dir DIR` | Use `DIR` instead of the platform default (also `SWOOP_DATA_DIR`). |
| `--socket PATH` | Unix socket path (default: `<data-dir>/swoop.sock`). |
| `--url URL` | Talk to a remote/local HTTP(S) server instead of the socket. |
| `--token TOKEN` | Bearer token for `--url`. |
| `--log-level LEVEL` | Log verbosity for `swoop server` (also `RUST_LOG`). |

If the engine isn't reachable, `swoop` prints `Swoop is not running. Start the app or run
`swoop server`.` and exits `3`.

**Exit codes:** `0` success · `1` error · `2` bad usage · `3` engine not running · `4` not found.

Every task/queue id accepted on the command line may be shortened to an unambiguous prefix of
the id shown by `swoop list` / `swoop queue list` (8 characters is usually enough); the full
UUID always works too. Queue arguments also accept a queue's name.

## Adding downloads

```
swoop add <URL|MAGNET|FILE.torrent>... [OPTIONS]
```

Each source becomes its own task (`swoop add a.zip b.zip` adds two tasks). A source is
classified automatically: `magnet:...` is a magnet link, a path ending in `.torrent` that exists
on disk is read and uploaded, everything else is a URL.

| Flag | Meaning |
|---|---|
| `--dir DIR` | Destination directory. |
| `--name NAME` | Override the file/task name. |
| `--queue ID\|NAME` | Target queue. |
| `--connections N` | Connections for this task. |
| `--limit BYTES` | Download limit in bytes/sec. |
| `--header 'Key: Value'` | Extra HTTP header (repeatable). |
| `--checksum algo:hex` | Expected checksum (`md5`, `sha1`, `sha256`, `sha512`, `blake3`). |
| `--paused` | Add without starting. |
| `--torrent FILE` | Add a `.torrent` file explicitly (same as a positional path). |
| `--magnet URI` | Add a magnet URI explicitly (same as a positional magnet). |

```
swoop add https://example.com/file.zip --dir ~/Downloads --connections 8
swoop add 'magnet:?xt=urn:btih:...' --paused
swoop add linux.iso.torrent --limit 2000000
```

## Inspecting and controlling tasks

```
swoop list [--state STATE]... [--queue ID|NAME] [--search TEXT] [--limit N]
swoop status [ID]                  # global stats, or one task's detail
swoop pause ID...
swoop resume ID...
swoop retry ID...                  # keep partial data
swoop restart ID...                # discard partial data
swoop cancel ID...
swoop remove ID... [--delete-file]
swoop pause-all
swoop resume-all
swoop watch [ID]                   # live progress; plain lines when stdout isn't a TTY
swoop diagnostics ID
```

`swoop status` with no id prints download/upload speed, active/queued/paused counts, today's
completed/failed counts, the traffic mode and the current limits. `swoop status ID` prints one
task's state, progress, queue, priority, tags and last error.

## Queues

```
swoop queue list
swoop queue create NAME [--max N]     # N = 0 means unlimited concurrency
swoop queue pause ID
swoop queue resume ID
swoop queue delete ID
```

## Bandwidth

```
swoop limit [--down BYTES] [--up BYTES]     # 0 = unlimited; omitted side is left as-is
swoop mode unlimited|balanced|browsing|custom
```

## History, export and import

```
swoop history [--search TEXT] [--limit N]
swoop export [--tasks] [--history] > backup.json
swoop import backup.json [--overwrite]
```

`swoop export` always writes raw JSON to stdout (regardless of `--json`) so it composes with
shell redirection. `swoop import` reads a bundle previously produced by `swoop export` and
imports whichever sections are present in the file.

## Remote pairing and devices

```
swoop pair                 # prints a pairing code and a QR code (admin only)
swoop devices               # list paired devices
swoop devices revoke ID
```

## Running the engine headlessly

```
swoop server [--listen ADDR] [--remote ADDR] [--no-tls] [--download-dir DIR]
swoop --headless           # shorthand for `swoop server` with defaults
```

Always starts the local Unix socket API. `--listen 127.0.0.1:41779` also opens a loopback TCP
listener (useful where Unix sockets aren't available). `--remote 0.0.0.0:41780` opens the
remote (LAN/Internet) listener for paired devices, with TLS **on by default** — pass `--no-tls`
only behind a trusted reverse proxy that terminates TLS itself; doing so prints a loud warning
and sends tokens in clear text otherwise. Logs go to stderr; control verbosity with
`--log-level` or `RUST_LOG`. `SIGINT`/`SIGTERM` trigger a graceful shutdown (transfers paused,
checkpoints flushed, socket removed).

## Browser native-messaging host

```
swoop native-host
swoop native-host --install-manifest all|chrome|chrome-beta|chrome-canary|chromium|brave|edge|vivaldi|arc|opera|firefox [--extension-id ID]
```

`swoop native-host` (no flags) is what the browser launches via Chrome/Firefox's native
messaging protocol — 4-byte little-endian length prefix + UTF-8 JSON on stdin/stdout, max 1 MiB
per message. It is a stateless relay to the local API over the Unix socket: it reads the local
token itself, refuses any path outside `/api/v1/` and the sensitive groups (`settings`,
`devices`, `automations`, `import`, `export`, `archives`, `plugins`), and forwards WebSocket
events to subscribed requests. `--install-manifest` writes the native-messaging host manifest
(`app.swoop.bridge.json`) to the right per-browser directory on macOS or Linux, pointing at the
current executable and allowing Swoop's fixed extension ids; `all` does this for every installed
browser, as the macOS app does on each launch. `--extension-id` additionally
allows another id (a self-built extension with its own key). You don't normally need this command:
the app keeps the manifests up to date.

## Docker

```
docker build -t swoop .
docker run -p 41780:41780 -v swoop-data:/data -v swoop-downloads:/downloads swoop
# or:
docker compose -f docker/compose.yaml up -d --build
```

The image runs `swoop server --remote 0.0.0.0:41780 --download-dir /downloads` as a non-root
user, with `SWOOP_DATA_DIR=/data`. `swoop` (the same binary, from another shell) can then
control it via `docker exec -it swoop swoop status --socket /data/swoop.sock`, or remotely
via `swoop pair --url https://host:41780` from a workstation with a paired token.
