//! The single librqbit [`Session`] behind the engine: construction from settings, lazy start,
//! and the handful of operations the run loop needs. librqbit spawns its own Tokio tasks on
//! the ambient runtime, so this must be created from inside the services runtime.

use crate::limits;
use librqbit::api::TorrentIdOrHash;
use librqbit::dht::DhtPersistenceConfig;
use librqbit::dht::Id20;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, Api, DhtSessionConfig, ListOnlyResponse,
    ListenerMode, ListenerOptions, ManagedTorrent, ManagedTorrentState, Session, SessionOptions,
    SessionPersistenceConfig,
};

/// librqbit hands out `Arc<ManagedTorrent>` but does not re-export its alias for it.
pub(crate) type ManagedTorrentHandle = Arc<ManagedTorrent>;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use swoop_domain::settings::Settings;
use swoop_domain::{ErrorKind, TaskError};
use swoop_runtime::redact::redact;
use tracing::{debug, info, warn};

/// Knobs that have no user-facing setting; tests use them to build isolated loopback swarms.
#[derive(Clone, Debug, Default)]
pub struct SessionTuning {
    /// Never talk to trackers (neither librqbit nor our probes).
    pub disable_trackers: bool,
    /// Do not multicast info hashes on the LAN (BEP-14).
    pub disable_local_service_discovery: bool,
    /// Peers handed to every torrent at add time.
    pub initial_peers: Vec<SocketAddr>,
    /// Bind address for the peer listener (default: unspecified v6/v4 per `ipv4_only`).
    pub listen_addr: Option<IpAddr>,
    /// Client name sent in extended handshakes and tracker User-Agent.
    pub client_name: Option<String>,
}

pub(crate) struct TorrentSession {
    pub session: Arc<Session>,
    api: Api,
    session_dir: PathBuf,
    pub tuning: SessionTuning,
}

impl TorrentSession {
    pub async fn start(
        session_dir: PathBuf,
        default_output: PathBuf,
        settings: &Settings,
        tuning: SessionTuning,
    ) -> Result<Arc<Self>, TaskError> {
        std::fs::create_dir_all(&session_dir)
            .map_err(|e| TaskError::from_io(&e, "creating torrent session directory"))?;
        let opts = session_options(&session_dir, settings, &tuning);
        let session = Session::new_with_opts(default_output, opts)
            .await
            .map_err(|e| classify("starting torrent session", &e))?;
        info!(
            listen = ?session.listen_addr(),
            dht = settings.torrent.dht,
            "torrent session started"
        );
        let this = Arc::new(Self {
            api: Api::new(session.clone(), None),
            session,
            session_dir,
            tuning,
        });
        this.park_restored_torrents().await;
        Ok(this)
    }

    /// librqbit restores every persisted torrent in the state it was in; the services layer is
    /// the only party allowed to start transfers, so park them all until `run()` adopts them.
    async fn park_restored_torrents(self: &Arc<Self>) {
        let restored: Vec<ManagedTorrentHandle> = self
            .session
            .with_torrents(|it| it.map(|(_, h)| h.clone()).collect());
        for handle in restored {
            debug!(info_hash = %handle.info_hash().as_string(), "parking restored torrent");
            if let Err(e) = self.session.pause(&handle).await {
                debug!("pause on restore: {e:#}");
            }
            // A hash check that finished between restore and our pause goes live anyway;
            // keep watching until the torrent settles or somebody legitimately starts it.
            let this = self.clone();
            tokio::spawn(async move {
                let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
                while tokio::time::Instant::now() < deadline {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    if !handle.is_paused() {
                        return; // started by run()
                    }
                    let settled = handle.with_state(|s| {
                        matches!(
                            s,
                            ManagedTorrentState::Paused(_) | ManagedTorrentState::Error(_)
                        )
                    });
                    if settled {
                        return;
                    }
                    if handle.live().is_some() {
                        if let Err(e) = this.session.pause(&handle).await {
                            debug!("late pause on restore: {e:#}");
                        }
                        return;
                    }
                }
            });
        }
    }

    pub fn listen_port(&self) -> Option<u16> {
        self.session.listen_addr().map(|a| a.port())
    }

    /// Port our tracker probes announce (the same one librqbit announces).
    pub fn announce_port(&self) -> u16 {
        self.session
            .announce_port()
            .or_else(|| self.listen_port())
            .unwrap_or(4240)
    }

    pub fn apply_global_limits(&self, download: u64, upload: u64) {
        self.session
            .ratelimits
            .set_download_bps(limits::bps(download));
        self.session.ratelimits.set_upload_bps(limits::bps(upload));
    }

    pub fn dht_nodes(&self) -> u32 {
        self.session
            .get_dht()
            .map(|d| {
                let s = d.stats();
                u32::try_from(s.routing_table_size + s.routing_table_size_v6).unwrap_or(u32::MAX)
            })
            .unwrap_or(0)
    }

    pub fn get(&self, hash: Id20) -> Option<ManagedTorrentHandle> {
        self.session.get(TorrentIdOrHash::Hash(hash))
    }

    pub async fn add(
        &self,
        bytes: Vec<u8>,
        opts: AddTorrentOptions,
    ) -> Result<AddTorrentResponse, TaskError> {
        self.session
            .add_torrent(AddTorrent::from_bytes(bytes), Some(opts))
            .await
            .map_err(|e| classify("adding torrent", &e))
    }

    /// Resolve a magnet's metadata without starting a download. Blocks until metadata arrives;
    /// callers wrap it in a timeout.
    pub async fn resolve_magnet(&self, uri: &str) -> Result<ListOnlyResponse, TaskError> {
        let opts = AddTorrentOptions {
            list_only: true,
            initial_peers: (!self.tuning.initial_peers.is_empty())
                .then(|| self.tuning.initial_peers.clone()),
            ..Default::default()
        };
        match self
            .session
            .add_torrent(AddTorrent::from_url(uri), Some(opts))
            .await
            .map_err(|e| classify("resolving magnet", &e))?
        {
            AddTorrentResponse::ListOnly(r) => Ok(r),
            AddTorrentResponse::AlreadyManaged(_, handle)
            | AddTorrentResponse::Added(_, handle) => {
                // Already in the session (restored or running): its metadata is what we want.
                let (info, torrent_bytes) = handle
                    .with_metadata(|m| (m.info.clone(), bytes::Bytes::clone(&m.torrent_bytes)))
                    .map_err(|e| classify("reading metadata", &e))?;
                Ok(ListOnlyResponse {
                    info_hash: handle.info_hash(),
                    info,
                    only_files: handle.only_files(),
                    output_folder: handle.output_folder().to_path_buf(),
                    seen_peers: Vec::new(),
                    torrent_bytes,
                })
            }
        }
    }

    /// Replace a managed torrent with a fresh add (new options) without losing its verified
    /// pieces: librqbit keeps the have-bitfield in `<session_dir>/<info_hash>.bitv` and deletes
    /// it on `delete`, so we snapshot it through the API and put it back before re-adding —
    /// fast-resume then samples a few pieces instead of re-hashing everything. If anything in
    /// that dance fails the torrent is simply re-checked in full (slower, never unsafe).
    pub async fn readd(
        &self,
        handle: &ManagedTorrentHandle,
        bytes: Vec<u8>,
        opts: AddTorrentOptions,
    ) -> Result<ManagedTorrentHandle, TaskError> {
        let hash = handle.info_hash();
        if handle.live().is_some() {
            if let Err(e) = self.session.pause(handle).await {
                debug!("pause before re-add: {e:#}");
            }
        }
        let haves = self
            .api
            .api_dump_haves(TorrentIdOrHash::Hash(hash))
            .ok()
            .map(|(bf, _)| bf.as_raw_slice().to_vec());
        self.session
            .delete(TorrentIdOrHash::Hash(hash), false)
            .await
            .map_err(|e| classify("removing torrent for re-add", &e))?;
        if let Some(raw) = haves {
            let path = self.bitv_path(&hash);
            if let Err(e) = std::fs::write(&path, raw) {
                warn!(path = %path.display(), "could not preserve piece bitfield, full re-check follows: {e}");
            }
        }
        match self.add(bytes, opts).await? {
            AddTorrentResponse::Added(_, h) | AddTorrentResponse::AlreadyManaged(_, h) => Ok(h),
            AddTorrentResponse::ListOnly(_) => {
                Err(TaskError::internal("re-add returned list-only"))
            }
        }
    }

    fn bitv_path(&self, hash: &Id20) -> PathBuf {
        self.session_dir.join(format!("{}.bitv", hash.as_string()))
    }

    pub async fn pause(&self, handle: &ManagedTorrentHandle) -> Result<(), TaskError> {
        match self.session.pause(handle).await {
            Ok(()) => Ok(()),
            Err(e) if e.to_string().contains("already paused") => Ok(()),
            Err(e) => Err(classify("pausing torrent", &e)),
        }
    }

    pub async fn unpause(&self, handle: &ManagedTorrentHandle) -> Result<(), TaskError> {
        match self.session.unpause(handle).await {
            Ok(()) => Ok(()),
            Err(e) if e.to_string().contains("already live") => Ok(()),
            Err(e) => Err(classify("resuming torrent", &e)),
        }
    }

    pub async fn update_only_files(
        &self,
        handle: &ManagedTorrentHandle,
        only: &std::collections::HashSet<usize>,
    ) -> Result<(), TaskError> {
        self.session
            .update_only_files(handle, only)
            .await
            .map_err(|e| classify("updating file selection", &e))
    }

    pub async fn delete(&self, hash: Id20, delete_files: bool) -> Result<(), TaskError> {
        match self
            .session
            .delete(TorrentIdOrHash::Hash(hash), delete_files)
            .await
        {
            Ok(()) => Ok(()),
            Err(e) if e.to_string().contains("no such torrent") => Ok(()),
            Err(e) => Err(classify("removing torrent", &e)),
        }
    }

    /// Have-bitfield as booleans plus the piece count (works while paused or live).
    pub fn haves(&self, hash: Id20) -> Option<(Vec<bool>, u32)> {
        let (bf, len) = self.api.api_dump_haves(TorrentIdOrHash::Hash(hash)).ok()?;
        let bits: Vec<bool> = bf.iter().map(|b| *b).take(len as usize).collect();
        Some((bits, len))
    }

    pub async fn stop(&self) {
        self.session.stop().await;
    }
}

pub(crate) fn session_options(
    session_dir: &Path,
    settings: &Settings,
    tuning: &SessionTuning,
) -> SessionOptions {
    let t = &settings.torrent;
    let ipv4_only = settings.network.ipv4_only;
    let bind_ip = tuning.listen_addr.unwrap_or(if ipv4_only {
        IpAddr::V4(Ipv4Addr::UNSPECIFIED)
    } else {
        IpAddr::V6(Ipv6Addr::UNSPECIFIED)
    });
    SessionOptions {
        dht: t.dht.then(|| DhtSessionConfig {
            bootstrap_addrs: None,
            port: None,
            persistence: Some(DhtPersistenceConfig {
                dump_interval: None,
                config_filename: Some(session_dir.join("dht.json")),
            }),
        }),
        disable_trackers: tuning.disable_trackers,
        fastresume: true,
        persistence: Some(SessionPersistenceConfig::Json {
            folder: Some(session_dir.to_path_buf()),
        }),
        listen: Some(ListenerOptions {
            mode: ListenerMode::TcpOnly,
            listen_addr: SocketAddr::new(bind_ip, t.listen_port),
            enable_upnp_port_forwarding: false,
            utp_opts: None,
            announce_port: None,
            ipv4_only,
            max_pending_incoming_handshake_checks: 256,
        }),
        ratelimits: limits::limits_config(t.download_limit, t.upload_limit),
        peer_limit: Some(usize::try_from(t.max_peers_per_torrent.max(1)).unwrap_or(usize::MAX)),
        disable_upload: limits::upload_disabled(t),
        disable_local_service_discovery: tuning.disable_local_service_discovery,
        ipv4_only,
        client_name_and_version: Some(
            tuning
                .client_name
                .clone()
                .unwrap_or_else(|| format!("Swoop {}", env!("CARGO_PKG_VERSION"))),
        ),
        ..Default::default()
    }
}

/// Map librqbit's `anyhow` errors onto the domain taxonomy by inspecting the message chain.
pub(crate) fn classify(context: &str, err: &anyhow::Error) -> TaskError {
    let chain = err
        .chain()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join(": ");
    let lower = chain.to_ascii_lowercase();
    let kind = if lower.contains("path traversal")
        || lower.contains("decoding torrent")
        || lower.contains("bencode")
        || lower.contains("not a valid magnet")
        || lower.contains("infohash")
        || lower.contains("invalid torrent")
        || lower.contains("out of range")
    {
        ErrorKind::InvalidTorrent
    } else if lower.contains("no known way to resolve peers") {
        ErrorKind::DhtUnavailable
    } else if lower.contains("stream exhausted") || lower.contains("no way to discover") {
        ErrorKind::NoPeers
    } else if lower.contains("permission denied") || lower.contains("read-only") {
        ErrorKind::PermissionDenied
    } else if lower.contains("no space left") {
        ErrorKind::DiskFull
    } else if lower.contains("address already in use") || lower.contains("error starting listeners")
    {
        ErrorKind::NetworkUnavailable
    } else if lower.contains("error opening")
        || lower.contains("error creating")
        || lower.contains("error writing")
    {
        ErrorKind::DiskWriteError
    } else {
        ErrorKind::Unknown
    };
    TaskError::new(kind, format!("{context}: {}", redact(&chain)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_errors() {
        let e = anyhow::anyhow!("error decoding torrent").context("adding");
        assert_eq!(classify("x", &e).kind, ErrorKind::InvalidTorrent);
        let e = anyhow::anyhow!(
            "no known way to resolve peers (no DHT, no trackers, no initial_peers)"
        );
        assert_eq!(classify("x", &e).kind, ErrorKind::DhtUnavailable);
        let e =
            anyhow::anyhow!("input address stream exhausted, no way to discover torrent metainfo");
        assert_eq!(classify("x", &e).kind, ErrorKind::NoPeers);
        let e = anyhow::anyhow!("something odd");
        assert_eq!(classify("x", &e).kind, ErrorKind::Unknown);
        let e = anyhow::anyhow!("https://user:pw@host/x failed");
        assert!(!classify("x", &e).message.contains("pw@"));
    }

    #[test]
    fn options_follow_settings() {
        let mut settings = Settings::default();
        settings.torrent.dht = false;
        settings.torrent.listen_port = 4321;
        settings.torrent.download_limit = 1000;
        settings.torrent.seed_when_complete = false;
        settings.torrent.seed_ratio_limit = 0.0;
        let opts = session_options(Path::new("/tmp/s"), &settings, &SessionTuning::default());
        assert!(opts.dht.is_none());
        assert!(opts.fastresume);
        assert!(opts.disable_upload);
        assert_eq!(
            opts.listen.as_ref().map(|l| l.listen_addr.port()),
            Some(4321)
        );
        assert_eq!(opts.ratelimits.download_bps.map(|n| n.get()), Some(1000));
        assert_eq!(opts.peer_limit, Some(120));
    }
}
