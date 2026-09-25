# swoop-engine-torrent

BitTorrent engine for Swoop, built on [librqbit](https://github.com/ikatson/rqbit) 9. It implements
`swoop_runtime::engine::Transfer` for `TaskKind::Torrent` and `TaskKind::Magnet` and exposes the
control surface the services layer needs (file selection, seeding limits, trackers, peers, live
info).

## Public API

| Item | Purpose |
|---|---|
| `TorrentEngine::new(TorrentEngineConfig) -> Result<Arc<TorrentEngine>>` | Cheap; the librqbit session starts lazily on first use. Construct it inside the services Tokio runtime — librqbit spawns its tasks on the ambient runtime. |
| `TorrentEngineConfig { session_dir, default_output, settings, blobs: Arc<dyn TorrentBlobProvider>, http: reqwest::Client, tuning: SessionTuning }` | `session_dir` = `AppPaths::torrent_session_dir()`; `http` should come from `ClientFactory::default_client()`; `tuning` holds test-only knobs (disable trackers/LSD, inject peers, bind address). |
| `trait TorrentBlobProvider { async fn torrent_bytes(&self, info_hash) -> Option<Vec<u8>> }` | The services layer implements it over `torrent_blobs`. |
| `Transfer::run / probe / forget` | Task lifecycle. `forget(task, delete_files)` maps to `Session::delete`. |
| `listen_port() -> Option<u16>` | Actual peer port once the session is up (`0` in settings = random). |
| `apply_settings(&Settings) -> DeferredSettings` | Global rate limits and `additional_trackers` apply live; DHT, listen port, IPv4-only and the leech-only switch need a restart and are listed in `DeferredSettings::keys`. |
| `shutdown()` | Pauses every torrent (persisting state) and stops the session. Stop tasks first. |
| `TorrentEngine::parse_torrent(&[u8]) -> Result<TorrentInfo>` | Add dialog; no session needed. |
| `resolve_magnet(uri, timeout) -> Result<(TorrentInfo, Vec<u8>)>` | List-only metadata fetch; returns metainfo bytes so the task can become a `.torrent` task. Times out with `NoPeers`; no peer source at all gives `DhtUnavailable`. |
| `set_seeding_limits(task_id, SeedingLimits)` | Live override (engine override → task options → settings). |
| `trackers(task_id) -> Vec<TrackerStatus>` / `add_trackers` / `remove_tracker` / `set_tracker_enabled` / `reannounce` | Tracker registry + probes (see below). |
| `refresh_tracker_list(url) -> Result<u32>` | Fetch a newline-separated list over HTTP(S), apply to loaded public torrents, remember it for later adds; returns the number of valid URLs. |
| `peers(task_id) -> Vec<PeerInfo>` | Live peers with speeds derived between calls. |
| `torrent_info(task_id) -> Option<TorrentInfo>` | Static description merged with live counters, tracker rows, DHT node count and the piece map. |

Sequential toggles, per-task limits and file selection arrive through `TransferControl`
(`sequential`, `download_limit`/`upload_limit`, `set_file_selection`) and are polled every second.

## How the spec maps onto librqbit

| Feature | librqbit | Notes |
|---|---|---|
| Session | `Session::new_with_opts` with JSON persistence + `fastresume` in `session_dir`, DHT per `settings.torrent.dht` (routing table persisted to `session_dir/dht.json`), TCP listener on `settings.torrent.listen_port`, `peer_limit` from `max_peers_per_torrent`, global `ratelimits` from `download_limit`/`upload_limit`, `disable_upload` when `seed_when_complete == false && seed_ratio_limit == 0` (needs the `disable-upload` cargo feature, enabled here). | librqbit restores persisted torrents *running*; the engine pauses them all on start so only `run()` starts transfers (the recovery table drives that). |
| Adding a `.torrent` | `AddTorrent::from_bytes` with `output_folder = task.directory[/name]`, `only_files`, `overwrite: true`, per-torrent `ratelimits`, `peer_limit`, `initial_peers`. | The metainfo is **re-wrapped** (`metainfo::wrap_with_trackers`): the raw `info` dict is kept byte-for-byte (same info hash) and the announce-list is replaced by the registry's enabled trackers. That is the only way to make librqbit honour disabled/added trackers. |
| Magnet | `add_torrent(list_only: true)` resolves metadata (`Resolving` state), returns metainfo bytes + peers seen; the real add then uses those bytes and `initial_peers`. If the session already holds the hash (restored), its metadata is reused without a network round-trip. | |
| Resume / adopt | `AddTorrentResponse::AlreadyManaged` or `Session::get(hash)` → `update_only_files` + `unpause`. | If output folder, tracker set, per-torrent limits or peer cap differ the torrent is re-added (below). |
| Live option changes | librqbit fixes per-torrent rate limits, peer cap and trackers at add time. The engine re-adds the torrent (debounced 2 s): `pause` → `Api::api_dump_haves` → `delete(keep files)` → write `<hash>.bitv` back → `add`. Fast-resume validates a sample of pieces instead of re-hashing. | Verified by `live_limit_change_readds_without_losing_pieces`. If the bitfield cannot be written the torrent is simply re-checked in full. |
| Progress | `ManagedTorrent::stats()` + `live().down/up_speed_estimator()` + `stats_snapshot().peer_stats`. | `total`/`downloaded` are piece-granular for partial selections (librqbit's `HaveNeededSelected`). During a hash check the last real figure is reported instead of "checked bytes". |
| Checkpoint | `Checkpoint::Torrent` every 10 s and on pause/stop/limit: selection, priorities, uploaded (accumulated across runs and re-adds), seeding_since, sequential flag, output folder, disabled + user trackers. | librqbit owns piece state in `session_dir`. |
| Seeding | `stats().finished` → `Seeding`; `SeedingPolicy` (ratio `0` = stop when complete, `< 0` = forever, `> 0` = target; time limit in minutes, `0` = none; `seed_when_complete`). Reaching a limit pauses the torrent and returns `Completed { file_path: root, bytes }`. | `run()` never returns `Seeding`; it stays alive until a limit or `TransferControl` stops it. |
| File selection | `Session::update_only_files`. | Priorities (0/1/2) are persisted and shown but librqbit has no per-file priority; only selected/skipped is applied. |
| Sequential | librqbit always fetches pieces in path order (first and last piece of each file first, then ascending) and has no rarest-first mode. The `sequential` flag is recorded and logged, nothing is faked. | |
| Peers | `live().per_peer_stats_snapshot(Live)` → address, client, cumulative bytes, connection kind (`T`/`U`/`S` flags, `I` incoming); speeds by differencing samples. | No per-peer progress (bitfields are private) → `progress = 0.0`; connected *seeds* cannot be counted → `seeds = 0`. |
| Piece map / availability | `Api::api_dump_haves` → run-length encoded `[missing, have, missing, …]`; `availability` = our own have-fraction (lower bound). | Swarm-wide availability would need peer bitfields. |
| DHT nodes | `Session::get_dht().stats()` routing-table sizes (v4 + v6). | |
| `NoPeers` | Live, unfinished, zero live peers and zero DHT nodes for 10 minutes → pause + `Failed(NoPeers)`. | Tracker failures are logged, never fatal. |
| Forget | `Session::delete(hash, delete_files)`. | |

## Trackers

librqbit announces internally but exposes no per-tracker state. `trackers::TrackerRegistry` keeps a
`TrackerStatus` row per URL (tier, enabled, last/next announce, seeders, leechers, latency, last
error, health, consecutive failures) and a probe loop announces with `numwant=0` using librqbit's
own peer id and port, so trackers see a single peer:

- HTTP(S): `GET <announce>?<tracker's own query>&info_hash=…&peer_id=…&port=…&uploaded=…&downloaded=…&left=…&compact=1&numwant=0&key=…[&event=started]`, bencoded reply decoded by the crate's own decoder (`failure reason`, `interval`, `min interval`, `complete`, `incomplete`, compact/dict peers, `peers6`).
- UDP: BEP-15 connect + announce, one retransmit, 15 s overall budget.
- Schedule: on start, on `reannounce`, and every `max(interval, 5 min)` unless `announce_interval_seconds` overrides. Health: `updating` → `working` / `error` → `dead` after 3 consecutive failures (retried hourly). `disabled` rows are neither probed nor given to librqbit.
- Adding/removing/enabling trackers bumps a generation; the run loop re-adds the torrent so librqbit announces to the new set.
- `seeders_total` / `leechers_total` are the maximum over trackers (they describe overlapping swarms).

Tracker URLs are redacted (`swoop_runtime::redact`) before they reach logs.

## Private torrents (BEP-27)

- No trackers from `settings.torrent.additional_trackers`, curated lists or `add_trackers` — those
  calls return `PermissionDenied`. Only the metainfo's own trackers are used (disable/enable of those
  still works).
- librqbit never uses DHT, LSD or PEX for private torrents (checked in `make_peer_rx` and the peer
  connection code); the engine never overrides that.

## BEPs

3 (core), 5 (DHT), 7 (IPv6 peers), 9 (metadata exchange / magnets), 10 (extension protocol),
11 (PEX), 12 (multitracker tiers), 14 (LSD), 15 (UDP trackers), 20 (peer id), 23 (compact peers),
27 (private torrents), 47 (padding files), 53 (`so=` in magnets). Peer-level protocol support
(BEP 3/9/10/11) is librqbit's; BEP 6 (fast extension) is not implemented by librqbit.

## Limitations

- **No WebSocket / WebTorrent trackers** (`ws://`, `wss://` are rejected).
- **Sequential** is not a toggle (see above); **per-file priorities** are recorded only.
- `settings.torrent.pex` and `encryption_required` cannot be honoured: librqbit has no PEX switch and no
  protocol encryption (MSE/PE).
- Per-torrent limits, peer cap and tracker changes apply through a re-add (a few seconds of peer
  churn); the global limits change instantly.
- Restored torrents are parked on session start and only run when the services layer calls `run()`.
- uTP listening is off (librqbit marks it experimental); outgoing connections are TCP.
- Connected-seed counts, per-peer progress and swarm availability are unavailable (peer bitfields are
  private in librqbit).
- Progress totals for partial selections are piece-granular.
- `shutdown()` is final; create a new engine to continue.

## Tests

`cargo test -p swoop-engine-torrent` runs unit tests (bencode, metainfo + unsafe paths, tracker
registry, HTTP/UDP probes against in-process stubs, seeding policy, peers, RLE) and loopback swarm
tests with a second librqbit session: full download with byte-identical files, pause protocol +
checkpoint resume, file selection, seeding until a live limit applies, magnet resolution and clean
timeout, tracker-driven peer discovery with health reporting (axum HTTP stub + UDP stub), private
torrent policy, and live limit changes through the re-add path. Nothing touches the public internet.
