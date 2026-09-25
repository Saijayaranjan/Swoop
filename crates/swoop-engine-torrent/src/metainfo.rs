//! `.torrent` and magnet parsing into the domain [`TorrentInfo`], path safety checks, output
//! layout, and re-wrapping an `info` dictionary with a chosen tracker list.

use crate::trackers::bencode::{self, Value};
use crate::trackers::{HEALTH_DISABLED, HEALTH_UPDATING};
use librqbit::dht::Id20;
use librqbit::{ByteBuf, Magnet, ValidatedTorrentMetaV1Info};
use std::path::{Path, PathBuf};
use swoop_domain::torrent::{TorrentFile, TorrentInfo, TrackerStatus};
use swoop_domain::{ErrorKind, TaskError};
use swoop_runtime::redact::redact;
use swoop_runtime::safety::{sanitize_filename, sanitize_relative_path};

/// Everything we keep from a parsed `.torrent`.
#[derive(Clone, Debug)]
pub struct ParsedTorrent {
    pub info_hash: Id20,
    /// Static description (no live counters; every file selected, nothing downloaded).
    pub info: TorrentInfo,
    /// Raw bencoded `info` dictionary (hashing input; reused verbatim when re-wrapping).
    pub info_bytes: Vec<u8>,
    /// `(url, tier)` from `announce-list` (or `announce` as tier 0).
    pub trackers: Vec<(String, u32)>,
    pub multi_file: bool,
    /// Relative paths exactly as librqbit will write them (validated, unsanitised).
    pub file_paths: Vec<PathBuf>,
    /// BEP-47 padding files: created empty, never selected.
    pub padding: Vec<bool>,
    pub comment: Option<String>,
    pub created_by: Option<String>,
}

pub fn invalid(msg: impl Into<String>) -> TaskError {
    TaskError::new(ErrorKind::InvalidTorrent, msg)
}

/// Parse and validate `.torrent` bytes. Rejects unsafe paths (traversal, absolute, control
/// or bidi-override characters, excessive depth) with [`ErrorKind::InvalidTorrent`].
pub fn parse_torrent(bytes: &[u8]) -> Result<ParsedTorrent, TaskError> {
    let meta = librqbit::torrent_from_bytes(bytes).map_err(|e| {
        invalid(format!(
            "not a valid torrent file: {}",
            redact(&e.to_string())
        ))
    })?;
    let validated: ValidatedTorrentMetaV1Info<ByteBuf<'_>> = meta
        .info
        .data
        .clone()
        .validate()
        .map_err(|e| invalid(format!("invalid torrent info: {}", redact(&e.to_string()))))?;

    let raw_name = validated.name().map(|n| n.into_owned());
    let multi_file = validated.info().files.is_some();
    let mut files = Vec::new();
    let mut file_paths = Vec::new();
    let mut padding = Vec::new();
    for (index, fd) in validated.iter_file_details().enumerate() {
        let components: Vec<String> = fd
            .filename
            .iter_components()
            .map(|c| c.into_owned())
            .collect();
        let rel = validate_components(&components)?;
        let is_padding = fd.attrs().padding;
        files.push(TorrentFile {
            index: u32::try_from(index).map_err(|_| invalid("too many files"))?,
            path: rel.to_string_lossy().into_owned(),
            size: fd.len,
            downloaded: 0,
            selected: !is_padding,
            priority: 1,
        });
        file_paths.push(rel);
        padding.push(is_padding);
    }
    if multi_file {
        let name = raw_name.as_deref().unwrap_or_default();
        if name.is_empty() {
            return Err(invalid("multi-file torrent without a name"));
        }
        validate_components(std::slice::from_ref(&name.to_owned()))?;
    }

    let trackers = tracker_tiers(&meta);
    let display_name = match (&raw_name, files.first()) {
        (Some(n), _) => n.clone(),
        (None, Some(f)) => f.path.clone(),
        (None, None) => return Err(invalid("torrent has no files")),
    };
    let comment = meta
        .comment
        .as_ref()
        .map(|c| String::from_utf8_lossy(c.as_ref()).into_owned());
    let created_by = meta
        .created_by
        .as_ref()
        .map(|c| String::from_utf8_lossy(c.as_ref()).into_owned());
    let lengths = validated.lengths();
    let private = validated.info().private;
    let info = TorrentInfo {
        info_hash: meta.info_hash.as_string(),
        name: display_name.clone(),
        total_size: lengths.total_length(),
        piece_length: lengths.default_piece_length(),
        piece_count: lengths.total_pieces(),
        files,
        trackers: tracker_rows(&trackers, &[]),
        private,
        comment: comment.clone(),
        created_by: created_by.clone(),
        magnet: Some(magnet_uri(
            &meta.info_hash,
            Some(&display_name),
            trackers.iter().map(|(u, _)| u.as_str()),
        )),
        have_metadata: true,
        ..TorrentInfo::default()
    };
    Ok(ParsedTorrent {
        info_hash: meta.info_hash,
        info,
        info_bytes: meta.info.raw_bytes.as_ref().to_vec(),
        trackers,
        multi_file,
        file_paths,
        padding,
        comment,
        created_by,
    })
}

fn tracker_tiers(meta: &librqbit::TorrentMetaV1<ByteBuf<'_>>) -> Vec<(String, u32)> {
    let mut out: Vec<(String, u32)> = Vec::new();
    let mut push = |bytes: &[u8], tier: u32| {
        if let Ok(s) = std::str::from_utf8(bytes) {
            let s = s.trim();
            if !s.is_empty() && !out.iter().any(|(u, _)| u == s) {
                out.push((s.to_owned(), tier));
            }
        }
    };
    if meta.announce_list.iter().any(|t| !t.is_empty()) {
        for (tier, urls) in meta.announce_list.iter().enumerate() {
            for u in urls {
                push(u.as_ref(), u32::try_from(tier).unwrap_or(u32::MAX));
            }
        }
    } else if let Some(a) = &meta.announce {
        push(a.as_ref(), 0);
    }
    out
}

/// Rows for a freshly parsed torrent (no probe results yet).
pub fn tracker_rows(trackers: &[(String, u32)], disabled: &[String]) -> Vec<TrackerStatus> {
    trackers
        .iter()
        .map(|(url, tier)| {
            let enabled = !disabled.contains(url);
            TrackerStatus {
                url: url.clone(),
                tier: *tier,
                enabled,
                last_announce_at: None,
                next_announce_at: None,
                seeders: None,
                leechers: None,
                latency_ms: None,
                last_error: None,
                health: if enabled {
                    HEALTH_UPDATING
                } else {
                    HEALTH_DISABLED
                }
                .into(),
                consecutive_failures: 0,
            }
        })
        .collect()
}

/// Validate one relative path given as components. Returns the path librqbit will write.
fn validate_components(components: &[String]) -> Result<PathBuf, TaskError> {
    let joined = components.join("/");
    sanitize_relative_path(&joined).map_err(|e| {
        invalid(format!("unsafe path in torrent: {}", e.message)).with_detail(redact(&joined))
    })?;
    let mut out = PathBuf::new();
    for c in components {
        if c.is_empty() || c == "." || c == ".." {
            return Err(invalid("unsafe path component in torrent").with_detail(redact(&joined)));
        }
        if c.chars().any(|ch| {
            ch.is_control()
                || matches!(
                    ch,
                    '\u{200B}'..='\u{200F}'
                        | '\u{202A}'..='\u{202E}'
                        | '\u{2066}'..='\u{2069}'
                        | '\u{FEFF}'
                )
        }) {
            return Err(
                invalid("torrent file name contains control or bidi-override characters")
                    .with_detail(redact(&joined)),
            );
        }
        if c.contains('/') || c.contains('\\') {
            return Err(
                invalid("torrent file name contains a path separator").with_detail(redact(&joined))
            );
        }
        out.push(c);
    }
    Ok(out)
}

/// Where librqbit writes and what the task's `file_path` is.
/// Multi-file torrents get `<directory>/<sanitised name>/…`; single-file torrents land as
/// `<directory>/<file name>`.
pub fn output_layout(directory: &Path, parsed: &ParsedTorrent) -> (PathBuf, PathBuf) {
    if parsed.multi_file {
        let root = directory.join(sanitize_filename(&parsed.info.name));
        (root.clone(), root)
    } else {
        let file = parsed
            .file_paths
            .first()
            .map(|p| directory.join(p))
            .unwrap_or_else(|| directory.to_path_buf());
        (directory.to_path_buf(), file)
    }
}

/// Description of a magnet link before its metadata is known.
pub struct MagnetPreview {
    pub info_hash: Id20,
    pub info: TorrentInfo,
    pub trackers: Vec<String>,
}

pub fn parse_magnet(uri: &str) -> Result<MagnetPreview, TaskError> {
    let magnet = Magnet::parse(uri).map_err(|e| {
        TaskError::new(
            ErrorKind::InvalidUrl,
            format!("invalid magnet link: {}", redact(&e.to_string())),
        )
    })?;
    let info_hash = magnet.as_id20().ok_or_else(|| {
        TaskError::new(
            ErrorKind::UnsupportedScheme,
            "magnet link has no BitTorrent v1 info hash (v2-only magnets are not supported)",
        )
    })?;
    let trackers: Vec<String> = magnet
        .trackers
        .iter()
        .filter(|t| crate::trackers::validate_tracker_url(t).is_ok())
        .cloned()
        .collect();
    let tiers: Vec<(String, u32)> = trackers.iter().map(|t| (t.clone(), 0)).collect();
    let info = TorrentInfo {
        info_hash: info_hash.as_string(),
        name: magnet.name.clone().unwrap_or_else(|| info_hash.as_string()),
        trackers: tracker_rows(&tiers, &[]),
        magnet: Some(uri.to_owned()),
        have_metadata: false,
        ..TorrentInfo::default()
    };
    Ok(MagnetPreview {
        info_hash,
        info,
        trackers,
    })
}

/// Build a magnet URI (`xt`, `dn`, `tr`).
pub fn magnet_uri<'a>(
    info_hash: &Id20,
    name: Option<&str>,
    trackers: impl Iterator<Item = &'a str>,
) -> String {
    let mut s = format!("magnet:?xt=urn:btih:{}", info_hash.as_string());
    if let Some(n) = name {
        s.push_str("&dn=");
        s.extend(url::form_urlencoded::byte_serialize(n.as_bytes()));
    }
    for t in trackers {
        s.push_str("&tr=");
        s.extend(url::form_urlencoded::byte_serialize(t.as_bytes()));
    }
    s
}

/// Re-wrap an `info` dictionary with the given announce tiers (BEP-12). The `info` bytes are
/// copied verbatim so the info hash is unchanged; this is how disabled/added trackers reach
/// librqbit, which only reads trackers from the metainfo it is given.
pub fn wrap_with_trackers(
    info_bytes: &[u8],
    tiers: &[Vec<String>],
    comment: Option<&str>,
    created_by: Option<&str>,
) -> Vec<u8> {
    let mut entries: Vec<(&str, Value)> = Vec::new();
    let first = tiers.iter().flatten().next();
    if let Some(first) = first {
        entries.push(("announce", bencode::bytes(first)));
    }
    let tier_values: Vec<Value> = tiers
        .iter()
        .filter(|t| !t.is_empty())
        .map(|t| Value::List(t.iter().map(|u| bencode::bytes(u)).collect()))
        .collect();
    if !tier_values.is_empty() {
        entries.push(("announce-list", Value::List(tier_values)));
    }
    if let Some(c) = comment {
        entries.push(("comment", bencode::bytes(c)));
    }
    if let Some(c) = created_by {
        entries.push(("created by", bencode::bytes(c)));
    }
    entries.push(("info", Value::Raw(info_bytes.to_vec())));
    bencode::encode(&bencode::dict(entries))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-build a torrent: `files` are `(path components, length)`.
    pub(crate) fn build_torrent(
        name: &str,
        files: &[(&[&str], u64)],
        private: bool,
        announce_list: &[&[&str]],
    ) -> Vec<u8> {
        let piece_length = 16 * 1024u64;
        let total: u64 = files.iter().map(|(_, l)| l).sum();
        let pieces = total.div_ceil(piece_length);
        let mut info = vec![
            ("name", bencode::bytes(name)),
            ("piece length", Value::Int(piece_length as i64)),
            ("pieces", Value::Bytes(vec![0xAB; (pieces * 20) as usize])),
        ];
        if files.len() == 1 && files[0].0.is_empty() {
            info.push(("length", Value::Int(files[0].1 as i64)));
        } else {
            let list = files
                .iter()
                .map(|(comps, len)| {
                    bencode::dict(vec![
                        ("length", Value::Int(*len as i64)),
                        (
                            "path",
                            Value::List(comps.iter().map(|c| bencode::bytes(c)).collect()),
                        ),
                    ])
                })
                .collect();
            info.push(("files", Value::List(list)));
        }
        if private {
            info.push(("private", Value::Int(1)));
        }
        let mut top = vec![
            ("comment", bencode::bytes("hello")),
            ("created by", bencode::bytes("test")),
        ];
        if let Some(first) = announce_list.iter().flat_map(|t| t.iter()).next() {
            top.push(("announce", bencode::bytes(first)));
            top.push((
                "announce-list",
                Value::List(
                    announce_list
                        .iter()
                        .map(|t| Value::List(t.iter().map(|u| bencode::bytes(u)).collect()))
                        .collect(),
                ),
            ));
        }
        top.push(("info", bencode::dict(info)));
        bencode::encode(&bencode::dict(top))
    }

    #[test]
    fn parses_multi_file_torrent() {
        let bytes = build_torrent(
            "My Torrent",
            &[(&["a.txt"], 100), (&["sub", "b.bin"], 40_000)],
            false,
            &[
                &["http://t1/announce", "http://t2/announce"],
                &["udp://t3:80/announce"],
            ],
        );
        let p = parse_torrent(&bytes).unwrap();
        assert!(p.multi_file);
        assert_eq!(p.info.name, "My Torrent");
        assert_eq!(p.info.total_size, 40_100);
        assert_eq!(p.info.piece_length, 16 * 1024);
        assert_eq!(p.info.piece_count, 3);
        assert_eq!(p.info.files.len(), 2);
        assert_eq!(p.info.files[1].path, "sub/b.bin");
        assert_eq!(p.trackers.len(), 3);
        assert_eq!(p.trackers[2], ("udp://t3:80/announce".to_string(), 1));
        assert_eq!(p.info.comment.as_deref(), Some("hello"));
        assert_eq!(p.info.created_by.as_deref(), Some("test"));
        assert!(!p.info.private);
        assert!(p.info.have_metadata);
        assert!(p.info.magnet.as_deref().unwrap().contains("dn=My+Torrent"));
        let (folder, root) = output_layout(Path::new("/tmp/dl"), &p);
        assert_eq!(folder, PathBuf::from("/tmp/dl/My Torrent"));
        assert_eq!(root, folder);
    }

    #[test]
    fn parses_single_file_and_private() {
        let bytes = build_torrent(
            "file.iso",
            &[(&[], 5000)],
            true,
            &[&["https://p/announce?passkey=x"]],
        );
        let p = parse_torrent(&bytes).unwrap();
        assert!(!p.multi_file);
        assert!(p.info.private);
        assert_eq!(p.info.files[0].path, "file.iso");
        let (folder, root) = output_layout(Path::new("/tmp/dl"), &p);
        assert_eq!(folder, PathBuf::from("/tmp/dl"));
        assert_eq!(root, PathBuf::from("/tmp/dl/file.iso"));
    }

    #[test]
    fn rejects_unsafe_paths() {
        for comps in [
            &["..", "etc", "passwd"][..],
            &["a\u{202E}txt.exe"],
            &["ok", "bad\u{0000}"],
            &["/abs"],
        ] {
            let bytes = build_torrent("t", &[(comps, 10), (&["fine"], 10)], false, &[]);
            let err = parse_torrent(&bytes).unwrap_err();
            assert_eq!(err.kind, ErrorKind::InvalidTorrent, "{comps:?}: {err}");
        }
        let deep: Vec<&str> = std::iter::repeat_n("d", 40).collect();
        let bytes = build_torrent("t", &[(&deep, 10), (&["fine"], 10)], false, &[]);
        assert_eq!(
            parse_torrent(&bytes).unwrap_err().kind,
            ErrorKind::InvalidTorrent
        );
        let bytes = build_torrent("..", &[(&["x"], 10), (&["y"], 10)], false, &[]);
        assert_eq!(
            parse_torrent(&bytes).unwrap_err().kind,
            ErrorKind::InvalidTorrent
        );
        assert_eq!(
            parse_torrent(b"garbage").unwrap_err().kind,
            ErrorKind::InvalidTorrent
        );
    }

    #[test]
    fn rewrap_keeps_info_hash_and_replaces_trackers() {
        let bytes = build_torrent(
            "t",
            &[(&["x"], 10), (&["y"], 10)],
            false,
            &[&["http://old/announce"]],
        );
        let p = parse_torrent(&bytes).unwrap();
        let tiers = vec![
            vec!["http://new1/announce".to_string()],
            vec!["udp://new2:1/announce".to_string()],
        ];
        let rewrapped = wrap_with_trackers(&p.info_bytes, &tiers, p.comment.as_deref(), None);
        let q = parse_torrent(&rewrapped).unwrap();
        assert_eq!(q.info_hash, p.info_hash);
        assert_eq!(
            q.trackers,
            vec![
                ("http://new1/announce".to_string(), 0),
                ("udp://new2:1/announce".to_string(), 1)
            ]
        );
        let none = wrap_with_trackers(&p.info_bytes, &[], None, None);
        assert!(parse_torrent(&none).unwrap().trackers.is_empty());
    }

    #[test]
    fn magnet_preview() {
        let m = parse_magnet(
            "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Name&tr=udp%3A%2F%2Ft%3A80%2Fannounce&tr=wss%3A%2F%2Fno",
        )
        .unwrap();
        assert_eq!(m.info.name, "Name");
        assert_eq!(m.trackers, vec!["udp://t:80/announce".to_string()]);
        assert!(!m.info.have_metadata);
        assert!(parse_magnet("magnet:?dn=nohash").is_err());
        assert!(parse_magnet("http://not-magnet").is_err());
    }
}
