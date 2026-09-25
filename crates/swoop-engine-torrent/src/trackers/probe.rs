//! Types shared by the HTTP and UDP tracker probes, plus the dispatcher that picks one by
//! URL scheme.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::{Duration, Instant};

/// How long a single probe (DNS + connect + announce) may take before it counts as failed.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnnounceEvent {
    None,
    Started,
    Stopped,
    Completed,
}

impl AnnounceEvent {
    pub fn as_http(self) -> Option<&'static str> {
        match self {
            AnnounceEvent::None => None,
            AnnounceEvent::Started => Some("started"),
            AnnounceEvent::Stopped => Some("stopped"),
            AnnounceEvent::Completed => Some("completed"),
        }
    }
    /// BEP-15 event code.
    pub fn as_udp(self) -> u32 {
        match self {
            AnnounceEvent::None => 0,
            AnnounceEvent::Completed => 1,
            AnnounceEvent::Started => 2,
            AnnounceEvent::Stopped => 3,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AnnounceRequest {
    pub info_hash: [u8; 20],
    pub peer_id: [u8; 20],
    pub port: u16,
    pub uploaded: u64,
    pub downloaded: u64,
    pub left: u64,
    pub event: AnnounceEvent,
    /// Random per-session key so trackers can match announces across IP changes (BEP-15
    /// `key`, HTTP `key=`).
    pub key: u32,
    /// Peers requested; health probes ask for none because librqbit runs its own announces.
    pub numwant: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AnnounceResponse {
    pub interval: Option<u64>,
    pub min_interval: Option<u64>,
    pub seeders: Option<u32>,
    pub leechers: Option<u32>,
    pub peers: Vec<SocketAddr>,
    pub warning: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeError {
    /// Already redacted; safe to log and show.
    pub message: String,
    /// The tracker explicitly refused us (failure reason, 4xx); retrying sooner will not help.
    pub permanent: bool,
}

impl ProbeError {
    pub fn transient(message: impl Into<String>) -> Self {
        Self {
            message: swoop_runtime::redact::redact(&message.into()),
            permanent: false,
        }
    }
    pub fn permanent(message: impl Into<String>) -> Self {
        Self {
            message: swoop_runtime::redact::redact(&message.into()),
            permanent: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProbeOutcome {
    pub result: Result<AnnounceResponse, ProbeError>,
    pub latency: Duration,
}

/// Announce to one tracker URL, dispatching on the scheme. WebSocket trackers (`ws`/`wss`)
/// are not supported and fail permanently.
pub async fn probe(url: &str, request: &AnnounceRequest, http: &reqwest::Client) -> ProbeOutcome {
    let started = Instant::now();
    let fut = async {
        let parsed = url::Url::parse(url).map_err(|e| ProbeError::permanent(e.to_string()))?;
        match parsed.scheme() {
            "http" | "https" => super::http::announce(http, &parsed, request).await,
            "udp" => super::udp::announce(&parsed, request).await,
            other => Err(ProbeError::permanent(format!(
                "unsupported tracker scheme {other:?}"
            ))),
        }
    };
    let result = match tokio::time::timeout(PROBE_TIMEOUT, fut).await {
        Ok(r) => r,
        Err(_) => Err(ProbeError::transient(format!(
            "tracker did not answer within {}s",
            PROBE_TIMEOUT.as_secs()
        ))),
    };
    ProbeOutcome {
        result,
        latency: started.elapsed(),
    }
}

/// Decode BEP-23 compact IPv4 peers (6 bytes each).
pub fn compact_peers_v4(buf: &[u8]) -> Vec<SocketAddr> {
    buf.as_chunks::<6>()
        .0
        .iter()
        .map(|c| {
            let ip = Ipv4Addr::new(c[0], c[1], c[2], c[3]);
            SocketAddr::from((ip, u16::from_be_bytes([c[4], c[5]])))
        })
        .collect()
}

/// Decode BEP-7 compact IPv6 peers (18 bytes each).
pub fn compact_peers_v6(buf: &[u8]) -> Vec<SocketAddr> {
    buf.as_chunks::<18>()
        .0
        .iter()
        .map(|c| {
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&c[..16]);
            SocketAddr::from((Ipv6Addr::from(octets), u16::from_be_bytes([c[16], c[17]])))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_decoding() {
        let v4 = compact_peers_v4(&[127, 0, 0, 1, 0x1a, 0xe1, 10, 0, 0, 2, 0, 80, 9]);
        assert_eq!(v4.len(), 2);
        assert_eq!(v4[0], "127.0.0.1:6881".parse().unwrap());
        assert_eq!(v4[1], "10.0.0.2:80".parse().unwrap());
        let mut v6 = vec![0u8; 16];
        v6[15] = 1;
        v6.extend_from_slice(&[0x1a, 0xe1]);
        assert_eq!(compact_peers_v6(&v6)[0], "[::1]:6881".parse().unwrap());
    }

    #[tokio::test]
    async fn rejects_unsupported_schemes() {
        let req = AnnounceRequest {
            info_hash: [0; 20],
            peer_id: [0; 20],
            port: 1,
            uploaded: 0,
            downloaded: 0,
            left: 0,
            event: AnnounceEvent::None,
            key: 0,
            numwant: 0,
        };
        let out = probe(
            "wss://tracker.example/announce",
            &req,
            &reqwest::Client::new(),
        )
        .await;
        let err = out.result.unwrap_err();
        assert!(err.permanent);
        assert!(err.message.contains("unsupported"));
    }
}
