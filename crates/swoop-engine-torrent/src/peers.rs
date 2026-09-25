//! Per-peer view built from librqbit's peer counters.
//!
//! librqbit exposes cumulative byte counters, connection kind and client name per peer, but
//! no per-peer speed and no peer bitfield. Speeds are derived here by differencing counters
//! between samples; `progress` is therefore always `0.0` (documented limitation).

use std::collections::HashMap;
use std::time::Instant;
use swoop_domain::torrent::PeerInfo;

#[derive(Clone, Copy)]
struct Sample {
    at: Instant,
    fetched: u64,
    uploaded: u64,
}

/// One per torrent; keeps the previous counters of every peer to compute speeds.
#[derive(Default)]
pub struct PeerSampler {
    last: HashMap<String, Sample>,
}

/// The subset of librqbit's `PeerStats` we read (kept as plain data so the sampler is
/// testable without a live torrent).
pub struct RawPeer {
    pub address: String,
    pub client: Option<String>,
    pub fetched_bytes: u64,
    pub uploaded_bytes: u64,
    pub incoming: bool,
    /// `tcp`, `utp` or `socks` as librqbit names them.
    pub connection_kind: Option<String>,
}

impl PeerSampler {
    pub fn sample(
        &mut self,
        now: Instant,
        peers: impl IntoIterator<Item = RawPeer>,
    ) -> Vec<PeerInfo> {
        let mut out = Vec::new();
        let mut fresh = HashMap::new();
        for p in peers {
            let (down, up) = match self.last.get(&p.address) {
                Some(prev) => {
                    let secs = now.saturating_duration_since(prev.at).as_secs_f64();
                    if secs > 0.05 {
                        (
                            (p.fetched_bytes.saturating_sub(prev.fetched) as f64 / secs) as u64,
                            (p.uploaded_bytes.saturating_sub(prev.uploaded) as f64 / secs) as u64,
                        )
                    } else {
                        (0, 0)
                    }
                }
                None => (0, 0),
            };
            fresh.insert(
                p.address.clone(),
                Sample {
                    at: now,
                    fetched: p.fetched_bytes,
                    uploaded: p.uploaded_bytes,
                },
            );
            out.push(PeerInfo {
                address: p.address,
                client: p.client,
                download_speed: down,
                upload_speed: up,
                progress: 0.0,
                flags: flags(p.incoming, p.connection_kind.as_deref(), down > 0, up > 0),
                downloaded: p.fetched_bytes,
                uploaded: p.uploaded_bytes,
            });
        }
        self.last = fresh;
        out.sort_by(|a, b| a.address.cmp(&b.address));
        out
    }
}

/// Compact flag string in the style other clients use: `I` incoming, `T`/`U`/`S` transport
/// (TCP/uTP/SOCKS), `D` downloading from, `u` uploading to.
fn flags(incoming: bool, kind: Option<&str>, downloading: bool, uploading: bool) -> String {
    let mut f = String::new();
    if incoming {
        f.push('I');
    }
    match kind.map(|k| k.to_ascii_lowercase()).as_deref() {
        Some("tcp") => f.push('T'),
        Some("utp") => f.push('U'),
        Some("socks") => f.push('S'),
        _ => {}
    }
    if downloading {
        f.push('D');
    }
    if uploading {
        f.push('u');
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn raw(addr: &str, fetched: u64, uploaded: u64) -> RawPeer {
        RawPeer {
            address: addr.into(),
            client: Some("qBittorrent".into()),
            fetched_bytes: fetched,
            uploaded_bytes: uploaded,
            incoming: addr.starts_with("10."),
            connection_kind: Some("tcp".into()),
        }
    }

    #[test]
    fn derives_speeds_between_samples() {
        let mut s = PeerSampler::default();
        let t0 = Instant::now();
        let first = s.sample(
            t0,
            vec![raw("1.1.1.1:1", 1000, 0), raw("10.0.0.1:2", 0, 500)],
        );
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].download_speed, 0);
        assert_eq!(first[1].flags, "IT");
        let second = s.sample(
            t0 + Duration::from_secs(2),
            vec![raw("1.1.1.1:1", 3000, 0), raw("10.0.0.1:2", 0, 900)],
        );
        assert_eq!(second[0].download_speed, 1000);
        assert_eq!(second[0].flags, "TD");
        assert_eq!(second[1].upload_speed, 200);
        assert_eq!(second[1].flags, "ITu");
        assert_eq!(second[1].downloaded, 0);
        assert_eq!(second[1].uploaded, 900);
        // a peer that disappears is forgotten
        let third = s.sample(t0 + Duration::from_secs(3), vec![raw("1.1.1.1:1", 3000, 0)]);
        assert_eq!(third.len(), 1);
        assert!(!s.last.contains_key("10.0.0.1:2"));
    }
}
