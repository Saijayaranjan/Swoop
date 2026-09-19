//! HTTP(S) tracker announce (BEP-3 §"Trackers", BEP-23 compact peers, BEP-7 IPv6).

use super::bencode::{self, Value};
use super::probe::{
    compact_peers_v4, compact_peers_v6, AnnounceRequest, AnnounceResponse, ProbeError,
};
use osprey_runtime::redact::redact;
use std::fmt::Write as _;
use std::net::SocketAddr;

/// Trackers answer with a few hundred bytes; anything bigger is not a tracker.
const MAX_RESPONSE_BYTES: usize = 256 * 1024;

/// Percent-encode raw bytes the way trackers expect (`%XX` for everything but unreserved
/// characters). `url::form_urlencoded` would encode spaces as `+`, which some trackers reject.
pub fn encode_binary(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 3);
    for &b in bytes {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

/// Build the announce URL: the tracker's own query parameters (passkeys) are preserved and
/// ours are appended.
pub fn announce_url(base: &url::Url, req: &AnnounceRequest) -> url::Url {
    let mut q = String::new();
    let _ = write!(
        q,
        "info_hash={}&peer_id={}&port={}&uploaded={}&downloaded={}&left={}&compact=1&no_peer_id=1&numwant={}&key={:08x}",
        encode_binary(&req.info_hash),
        encode_binary(&req.peer_id),
        req.port,
        req.uploaded,
        req.downloaded,
        req.left,
        req.numwant,
        req.key,
    );
    if let Some(ev) = req.event.as_http() {
        let _ = write!(q, "&event={ev}");
    }
    let mut url = base.clone();
    let combined = match base.query() {
        Some(existing) if !existing.is_empty() => format!("{existing}&{q}"),
        _ => q,
    };
    url.set_query(Some(&combined));
    url
}

pub async fn announce(
    client: &reqwest::Client,
    base: &url::Url,
    req: &AnnounceRequest,
) -> Result<AnnounceResponse, ProbeError> {
    let url = announce_url(base, req);
    let response = client.get(url).send().await.map_err(|e| {
        ProbeError::transient(format!("request failed: {}", redact(&e.to_string())))
    })?;
    let status = response.status();
    if !status.is_success() {
        let permanent = status.is_client_error();
        let err = ProbeError {
            message: format!("tracker responded with HTTP {}", status.as_u16()),
            permanent,
        };
        return Err(err);
    }
    if let Some(len) = response.content_length() {
        if len > MAX_RESPONSE_BYTES as u64 {
            return Err(ProbeError::permanent("tracker response too large"));
        }
    }
    let body = response
        .bytes()
        .await
        .map_err(|e| ProbeError::transient(format!("error reading response: {e}")))?;
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(ProbeError::permanent("tracker response too large"));
    }
    parse_response(&body)
}

/// Decode a bencoded announce response. Tolerates trailing garbage after the dictionary.
pub fn parse_response(body: &[u8]) -> Result<AnnounceResponse, ProbeError> {
    let (value, _) = bencode::decode_prefix(body).map_err(|e| {
        ProbeError::transient(format!("tracker returned a non-bencoded response ({e})"))
    })?;
    if value.as_dict().is_none() {
        return Err(ProbeError::transient(
            "tracker response is not a dictionary",
        ));
    }
    if let Some(reason) = value.get("failure reason").and_then(Value::as_str) {
        return Err(ProbeError::permanent(format!("tracker failure: {reason}")));
    }
    let peers = value
        .get("peers")
        .map(decode_peers_v4)
        .unwrap_or_default()
        .into_iter()
        .chain(
            value
                .get("peers6")
                .and_then(Value::as_bytes)
                .map(compact_peers_v6)
                .unwrap_or_default(),
        )
        .collect();
    Ok(AnnounceResponse {
        interval: value
            .get_int("interval")
            .and_then(|i| u64::try_from(i).ok()),
        min_interval: value
            .get_int("min interval")
            .and_then(|i| u64::try_from(i).ok()),
        seeders: value
            .get_int("complete")
            .and_then(|i| u32::try_from(i).ok()),
        leechers: value
            .get_int("incomplete")
            .and_then(|i| u32::try_from(i).ok()),
        peers,
        warning: value.get("warning message").and_then(Value::as_str),
    })
}

/// `peers` is either a compact byte string or a list of `{ip, port}` dictionaries.
fn decode_peers_v4(v: &Value) -> Vec<SocketAddr> {
    match v {
        Value::Bytes(b) => compact_peers_v4(b),
        Value::List(items) => items
            .iter()
            .filter_map(|p| {
                let ip: std::net::IpAddr = p.get("ip")?.as_str()?.parse().ok()?;
                let port = u16::try_from(p.get_int("port")?).ok()?;
                Some(SocketAddr::from((ip, port)))
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trackers::probe::AnnounceEvent;

    fn req() -> AnnounceRequest {
        AnnounceRequest {
            info_hash: [0xAB; 20],
            peer_id: *b"-OS0100-abcdefghijkl",
            port: 6881,
            uploaded: 1,
            downloaded: 2,
            left: 3,
            event: AnnounceEvent::Started,
            key: 0xdeadbeef,
            numwant: 0,
        }
    }

    #[test]
    fn builds_announce_url_preserving_passkey() {
        let base = url::Url::parse("https://t.example/announce?passkey=SECRET").unwrap();
        let u = announce_url(&base, &req());
        let s = u.as_str();
        assert!(s.starts_with("https://t.example/announce?passkey=SECRET&info_hash=%AB%AB"));
        assert!(s.contains("&peer_id=-OS0100-abcdefghijkl"));
        assert!(s.contains("&port=6881&uploaded=1&downloaded=2&left=3&compact=1"));
        assert!(s.contains("&numwant=0&key=deadbeef&event=started"));
        assert_eq!(encode_binary(b" ~a"), "%20~a");
    }

    #[test]
    fn parses_compact_and_dict_peers() {
        let body = b"d8:completei5e10:incompletei7e8:intervali900e12:min intervali60e5:peers6:\x7f\x00\x00\x01\x1a\xe1e";
        let r = parse_response(body).unwrap();
        assert_eq!(r.seeders, Some(5));
        assert_eq!(r.leechers, Some(7));
        assert_eq!(r.interval, Some(900));
        assert_eq!(r.min_interval, Some(60));
        assert_eq!(r.peers, vec!["127.0.0.1:6881".parse().unwrap()]);

        let dict = b"d8:intervali1e5:peersld2:ip9:10.0.0.254:porti81eeee";
        let r = parse_response(dict).unwrap();
        assert_eq!(r.peers, vec!["10.0.0.25:81".parse().unwrap()]);
    }

    #[test]
    fn failure_reason_is_permanent_and_garbage_is_transient() {
        let e = parse_response(b"d14:failure reason11:bad passkeye").unwrap_err();
        assert!(e.permanent);
        assert!(e.message.contains("bad passkey"));
        let e = parse_response(b"<html>nope</html>").unwrap_err();
        assert!(!e.permanent);
    }
}
