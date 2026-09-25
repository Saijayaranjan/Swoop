//! UDP tracker announce (BEP-15): connect handshake, then announce. One socket per probe; the
//! overall deadline is enforced by [`super::probe::probe`].

use super::probe::{
    compact_peers_v4, compact_peers_v6, AnnounceRequest, AnnounceResponse, ProbeError,
};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::UdpSocket;

const PROTOCOL_ID: u64 = 0x0417_2710_1980;
const ACTION_CONNECT: u32 = 0;
const ACTION_ANNOUNCE: u32 = 1;
const ACTION_ERROR: u32 = 3;
/// Per-exchange wait before one retransmit; two exchanges × two attempts fit the 15 s probe
/// budget.
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(3);

pub async fn announce(
    url: &url::Url,
    req: &AnnounceRequest,
) -> Result<AnnounceResponse, ProbeError> {
    let host = url
        .host_str()
        .ok_or_else(|| ProbeError::permanent("udp tracker URL has no host"))?;
    let port = url
        .port()
        .ok_or_else(|| ProbeError::permanent("udp tracker URL has no port"))?;
    let addr = resolve(host, port).await?;
    let bind: SocketAddr = if addr.is_ipv4() {
        "0.0.0.0:0"
            .parse()
            .map_err(|_| ProbeError::transient("bind"))?
    } else {
        "[::]:0"
            .parse()
            .map_err(|_| ProbeError::transient("bind"))?
    };
    let sock = UdpSocket::bind(bind)
        .await
        .map_err(|e| ProbeError::transient(format!("udp bind failed: {e}")))?;
    announce_on(&sock, addr, req).await
}

/// Announce using an already-bound socket (tests inject a loopback socket).
pub async fn announce_on(
    sock: &UdpSocket,
    addr: SocketAddr,
    req: &AnnounceRequest,
) -> Result<AnnounceResponse, ProbeError> {
    let connection_id = connect(sock, addr).await?;
    let tid: u32 = rand::random();
    let packet = announce_packet(connection_id, tid, req);
    let reply = exchange(sock, addr, &packet, tid).await?;
    parse_announce(&reply, tid)
}

async fn resolve(host: &str, port: u16) -> Result<SocketAddr, ProbeError> {
    let mut addrs = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| ProbeError::transient(format!("dns lookup failed for {host}: {e}")))?;
    // Prefer IPv4: most UDP trackers are v4-only and a v6 route may be black-holed.
    let first = addrs.next();
    let v4 = std::iter::once(first)
        .flatten()
        .chain(addrs)
        .find(SocketAddr::is_ipv4);
    v4.or(first)
        .ok_or_else(|| ProbeError::transient(format!("no addresses for {host}")))
}

async fn connect(sock: &UdpSocket, addr: SocketAddr) -> Result<u64, ProbeError> {
    let tid: u32 = rand::random();
    let mut packet = Vec::with_capacity(16);
    packet.extend_from_slice(&PROTOCOL_ID.to_be_bytes());
    packet.extend_from_slice(&ACTION_CONNECT.to_be_bytes());
    packet.extend_from_slice(&tid.to_be_bytes());
    let reply = exchange(sock, addr, &packet, tid).await?;
    if reply.len() < 16 {
        return Err(ProbeError::transient("short connect response"));
    }
    let action = u32::from_be_bytes([reply[0], reply[1], reply[2], reply[3]]);
    if action != ACTION_CONNECT {
        return Err(ProbeError::transient(format!(
            "unexpected action {action} in connect response"
        )));
    }
    let mut cid = [0u8; 8];
    cid.copy_from_slice(&reply[8..16]);
    Ok(u64::from_be_bytes(cid))
}

fn announce_packet(connection_id: u64, tid: u32, req: &AnnounceRequest) -> Vec<u8> {
    let mut p = Vec::with_capacity(98);
    p.extend_from_slice(&connection_id.to_be_bytes());
    p.extend_from_slice(&ACTION_ANNOUNCE.to_be_bytes());
    p.extend_from_slice(&tid.to_be_bytes());
    p.extend_from_slice(&req.info_hash);
    p.extend_from_slice(&req.peer_id);
    p.extend_from_slice(&req.downloaded.to_be_bytes());
    p.extend_from_slice(&req.left.to_be_bytes());
    p.extend_from_slice(&req.uploaded.to_be_bytes());
    p.extend_from_slice(&req.event.as_udp().to_be_bytes());
    p.extend_from_slice(&0u32.to_be_bytes()); // ip: default (sender address)
    p.extend_from_slice(&req.key.to_be_bytes());
    p.extend_from_slice(&(req.numwant as i32).to_be_bytes());
    p.extend_from_slice(&req.port.to_be_bytes());
    p
}

/// Send `packet` and wait for a reply carrying `tid`, retransmitting once. Replies with an
/// error action are turned into a permanent [`ProbeError`].
async fn exchange(
    sock: &UdpSocket,
    addr: SocketAddr,
    packet: &[u8],
    tid: u32,
) -> Result<Vec<u8>, ProbeError> {
    let mut buf = vec![0u8; 2048];
    for _attempt in 0..2 {
        sock.send_to(packet, addr)
            .await
            .map_err(|e| ProbeError::transient(format!("udp send failed: {e}")))?;
        let deadline = tokio::time::Instant::now() + EXCHANGE_TIMEOUT;
        loop {
            let recv = tokio::time::timeout_at(deadline, sock.recv_from(&mut buf)).await;
            let (n, from) = match recv {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => {
                    return Err(ProbeError::transient(format!("udp receive failed: {e}")))
                }
                Err(_) => break, // retransmit
            };
            if from.ip() != addr.ip() || n < 8 {
                continue; // stray datagram
            }
            let reply = &buf[..n];
            let action = u32::from_be_bytes([reply[0], reply[1], reply[2], reply[3]]);
            let got_tid = u32::from_be_bytes([reply[4], reply[5], reply[6], reply[7]]);
            if got_tid != tid {
                continue;
            }
            if action == ACTION_ERROR {
                let msg = String::from_utf8_lossy(&reply[8..]).trim().to_owned();
                return Err(ProbeError::permanent(format!("tracker error: {msg}")));
            }
            return Ok(reply.to_vec());
        }
    }
    Err(ProbeError::transient("udp tracker did not respond"))
}

fn parse_announce(reply: &[u8], tid: u32) -> Result<AnnounceResponse, ProbeError> {
    if reply.len() < 20 {
        return Err(ProbeError::transient("short announce response"));
    }
    let word = |i: usize| u32::from_be_bytes([reply[i], reply[i + 1], reply[i + 2], reply[i + 3]]);
    if word(0) != ACTION_ANNOUNCE || word(4) != tid {
        return Err(ProbeError::transient("announce response mismatch"));
    }
    let interval = word(8);
    let leechers = word(12);
    let seeders = word(16);
    let rest = &reply[20..];
    // IPv6 trackers answer 18-byte entries; detect by divisibility when 6 does not fit.
    let peers = if rest.len().is_multiple_of(6) {
        compact_peers_v4(rest)
    } else if rest.len().is_multiple_of(18) {
        compact_peers_v6(rest)
    } else {
        Vec::new()
    };
    Ok(AnnounceResponse {
        interval: Some(u64::from(interval)),
        min_interval: None,
        seeders: Some(seeders),
        leechers: Some(leechers),
        peers,
        warning: None,
    })
}

#[cfg(test)]
pub(crate) mod test_stub {
    //! A tiny BEP-15 tracker used by the unit tests: answers connect and announce, and can be
    //! told to answer announces with an error.
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    pub struct UdpTrackerStub {
        pub addr: SocketAddr,
        pub announces: Arc<AtomicU32>,
        _task: tokio::task::JoinHandle<()>,
    }

    impl UdpTrackerStub {
        pub async fn start(error: Option<&'static str>) -> Self {
            let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let addr = sock.local_addr().unwrap();
            let announces = Arc::new(AtomicU32::new(0));
            let counter = announces.clone();
            let task = tokio::spawn(async move {
                let mut buf = vec![0u8; 2048];
                loop {
                    let Ok((n, from)) = sock.recv_from(&mut buf).await else {
                        return;
                    };
                    let p = &buf[..n];
                    if n < 16 {
                        continue;
                    }
                    let action = u32::from_be_bytes([p[8], p[9], p[10], p[11]]);
                    let tid = &p[12..16];
                    let mut reply = Vec::new();
                    match action {
                        ACTION_CONNECT => {
                            reply.extend_from_slice(&ACTION_CONNECT.to_be_bytes());
                            reply.extend_from_slice(tid);
                            reply.extend_from_slice(&0x1122_3344_5566_7788u64.to_be_bytes());
                        }
                        ACTION_ANNOUNCE => {
                            counter.fetch_add(1, Ordering::Relaxed);
                            if let Some(msg) = error {
                                reply.extend_from_slice(&ACTION_ERROR.to_be_bytes());
                                reply.extend_from_slice(tid);
                                reply.extend_from_slice(msg.as_bytes());
                            } else {
                                reply.extend_from_slice(&ACTION_ANNOUNCE.to_be_bytes());
                                reply.extend_from_slice(tid);
                                reply.extend_from_slice(&1800u32.to_be_bytes());
                                reply.extend_from_slice(&4u32.to_be_bytes()); // leechers
                                reply.extend_from_slice(&9u32.to_be_bytes()); // seeders
                                reply.extend_from_slice(&[127, 0, 0, 1, 0x1a, 0xe1]);
                            }
                        }
                        _ => continue,
                    }
                    let _ = sock.send_to(&reply, from).await;
                }
            });
            Self {
                addr,
                announces,
                _task: task,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_stub::UdpTrackerStub;
    use super::*;
    use crate::trackers::probe::AnnounceEvent;

    fn req() -> AnnounceRequest {
        AnnounceRequest {
            info_hash: [1; 20],
            peer_id: [2; 20],
            port: 6881,
            uploaded: 0,
            downloaded: 0,
            left: 100,
            event: AnnounceEvent::Started,
            key: 7,
            numwant: 0,
        }
    }

    #[tokio::test]
    async fn announces_against_local_stub() {
        let stub = UdpTrackerStub::start(None).await;
        let url = url::Url::parse(&format!("udp://{}/announce", stub.addr)).unwrap();
        let r = announce(&url, &req()).await.unwrap();
        assert_eq!(r.seeders, Some(9));
        assert_eq!(r.leechers, Some(4));
        assert_eq!(r.interval, Some(1800));
        assert_eq!(r.peers, vec!["127.0.0.1:6881".parse().unwrap()]);
        assert_eq!(stub.announces.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn surfaces_tracker_errors() {
        let stub = UdpTrackerStub::start(Some("torrent not registered")).await;
        let url = url::Url::parse(&format!("udp://{}/announce", stub.addr)).unwrap();
        let e = announce(&url, &req()).await.unwrap_err();
        assert!(e.permanent);
        assert!(e.message.contains("not registered"));
    }

    #[test]
    fn packet_layout() {
        let p = announce_packet(0xAA, 0xBB, &req());
        assert_eq!(p.len(), 98);
        assert_eq!(&p[..8], &0xAAu64.to_be_bytes());
        assert_eq!(&p[8..12], &1u32.to_be_bytes());
        assert_eq!(&p[12..16], &0xBBu32.to_be_bytes());
        assert_eq!(&p[16..36], &[1u8; 20]);
        assert_eq!(&p[80..84], &2u32.to_be_bytes()); // event started
        assert_eq!(&p[96..98], &6881u16.to_be_bytes());
    }
}
