//! Minimal, strict M3U8 parser covering what a downloader needs: master playlists (variants,
//! audio renditions) and media playlists (segments, byte ranges, keys, init map, end list).

use swoop_domain::{ErrorKind, TaskError};
use url::Url;

#[derive(Clone, Debug, PartialEq)]
pub struct Variant {
    pub uri: String,
    pub bandwidth: Option<u64>,
    pub average_bandwidth: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub codecs: Option<String>,
    pub frame_rate: Option<f32>,
    pub audio_group: Option<String>,
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rendition {
    pub kind: String,
    pub group_id: String,
    pub name: String,
    pub uri: Option<String>,
    pub language: Option<String>,
    pub default: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MasterPlaylist {
    pub variants: Vec<Variant>,
    pub renditions: Vec<Rendition>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyMethod {
    None,
    Aes128,
    SampleAes,
    Other(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    pub method: KeyMethod,
    pub uri: Option<String>,
    /// 16-byte IV if given explicitly.
    pub iv: Option<[u8; 16]>,
    pub key_format: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ByteRange {
    pub length: u64,
    pub offset: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub index: u32,
    pub uri: String,
    pub duration: f64,
    pub byte_range: Option<ByteRange>,
    pub key: Option<Key>,
    /// Media sequence number (used as the default AES IV).
    pub sequence: u64,
    pub discontinuity: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InitSegment {
    pub uri: String,
    pub byte_range: Option<ByteRange>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaPlaylist {
    pub segments: Vec<Segment>,
    pub init: Option<InitSegment>,
    pub target_duration: f64,
    pub media_sequence: u64,
    pub end_list: bool,
    pub playlist_type: Option<String>,
    pub total_duration: f64,
    /// Any key with a method we cannot handle (DRM) was seen.
    pub protected: bool,
    pub uses_aes128: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Playlist {
    Master(MasterPlaylist),
    Media(MediaPlaylist),
}

/// Parse a playlist; `base` resolves relative URIs.
pub fn parse(text: &str, base: &Url) -> Result<Playlist, TaskError> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    match lines.next() {
        Some(first) if first.starts_with("#EXTM3U") => {}
        _ => {
            return Err(TaskError::new(
                ErrorKind::ParseError,
                "not an M3U8 playlist (missing #EXTM3U)",
            ))
        }
    }
    if text.contains("#EXT-X-STREAM-INF") {
        parse_master(text, base).map(Playlist::Master)
    } else {
        parse_media(text, base).map(Playlist::Media)
    }
}

fn resolve(base: &Url, uri: &str) -> Result<String, TaskError> {
    base.join(uri)
        .map(|u| u.to_string())
        .map_err(|e| TaskError::new(ErrorKind::ParseError, format!("bad URI {uri:?}: {e}")))
}

/// Parse `KEY=VALUE,KEY="quoted, value"` attribute lists.
pub fn parse_attributes(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut key = String::new();
    let mut val = String::new();
    let mut in_key = true;
    let mut in_quotes = false;
    for c in s.chars() {
        if in_key {
            if c == '=' {
                in_key = false;
            } else if c == ',' {
                key.clear();
            } else {
                key.push(c);
            }
        } else if in_quotes {
            if c == '"' {
                in_quotes = false;
            } else {
                val.push(c);
            }
        } else if c == '"' {
            in_quotes = true;
        } else if c == ',' {
            out.push((key.trim().to_ascii_uppercase(), std::mem::take(&mut val)));
            key.clear();
            in_key = true;
        } else {
            val.push(c);
        }
    }
    if !key.is_empty() {
        out.push((key.trim().to_ascii_uppercase(), val));
    }
    out
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

fn parse_master(text: &str, base: &Url) -> Result<MasterPlaylist, TaskError> {
    let mut pl = MasterPlaylist::default();
    let mut pending: Option<Vec<(String, String)>> = None;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(rest) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            pending = Some(parse_attributes(rest));
        } else if let Some(rest) = line.strip_prefix("#EXT-X-MEDIA:") {
            let a = parse_attributes(rest);
            pl.renditions.push(Rendition {
                kind: attr(&a, "TYPE").unwrap_or("").to_owned(),
                group_id: attr(&a, "GROUP-ID").unwrap_or("").to_owned(),
                name: attr(&a, "NAME").unwrap_or("").to_owned(),
                uri: match attr(&a, "URI") {
                    Some(u) => Some(resolve(base, u)?),
                    None => None,
                },
                language: attr(&a, "LANGUAGE").map(str::to_owned),
                default: attr(&a, "DEFAULT")
                    .map(|v| v.eq_ignore_ascii_case("YES"))
                    .unwrap_or(false),
            });
        } else if line.starts_with('#') {
            continue;
        } else if let Some(a) = pending.take() {
            let (width, height) = attr(&a, "RESOLUTION")
                .and_then(|r| r.split_once('x'))
                .map(|(w, h)| (w.parse().ok(), h.parse().ok()))
                .unwrap_or((None, None));
            pl.variants.push(Variant {
                uri: resolve(base, line)?,
                bandwidth: attr(&a, "BANDWIDTH").and_then(|v| v.parse().ok()),
                average_bandwidth: attr(&a, "AVERAGE-BANDWIDTH").and_then(|v| v.parse().ok()),
                width,
                height,
                codecs: attr(&a, "CODECS").map(str::to_owned),
                frame_rate: attr(&a, "FRAME-RATE").and_then(|v| v.parse().ok()),
                audio_group: attr(&a, "AUDIO").map(str::to_owned),
                name: attr(&a, "NAME").map(str::to_owned),
            });
        }
    }
    if pl.variants.is_empty() {
        return Err(TaskError::new(
            ErrorKind::ParseError,
            "master playlist has no variants",
        ));
    }
    Ok(pl)
}

fn parse_iv(s: &str) -> Option<[u8; 16]> {
    let hex = s.trim_start_matches("0x").trim_start_matches("0X");
    if hex.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for i in 0..16 {
        out[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn parse_media(text: &str, base: &Url) -> Result<MediaPlaylist, TaskError> {
    let mut pl = MediaPlaylist::default();
    let mut key: Option<Key> = None;
    let mut duration: Option<f64> = None;
    let mut byte_range: Option<ByteRange> = None;
    let mut last_range_end: u64 = 0;
    let mut discontinuity = false;
    let mut index: u32 = 0;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let d = rest.split(',').next().unwrap_or("0").trim();
            duration = Some(d.parse().unwrap_or(0.0));
        } else if let Some(rest) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
            pl.target_duration = rest.trim().parse().unwrap_or(0.0);
        } else if let Some(rest) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            pl.media_sequence = rest.trim().parse().unwrap_or(0);
        } else if let Some(rest) = line.strip_prefix("#EXT-X-PLAYLIST-TYPE:") {
            pl.playlist_type = Some(rest.trim().to_owned());
        } else if line.starts_with("#EXT-X-ENDLIST") {
            pl.end_list = true;
        } else if line.starts_with("#EXT-X-DISCONTINUITY")
            && !line.starts_with("#EXT-X-DISCONTINUITY-SEQUENCE")
        {
            discontinuity = true;
        } else if let Some(rest) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            let (len, off) = match rest.split_once('@') {
                Some((l, o)) => (l.trim().parse().unwrap_or(0), o.trim().parse().unwrap_or(0)),
                None => (rest.trim().parse().unwrap_or(0), last_range_end),
            };
            byte_range = Some(ByteRange {
                length: len,
                offset: off,
            });
        } else if let Some(rest) = line.strip_prefix("#EXT-X-KEY:") {
            let a = parse_attributes(rest);
            let method = match attr(&a, "METHOD").unwrap_or("NONE") {
                "NONE" => KeyMethod::None,
                "AES-128" => KeyMethod::Aes128,
                "SAMPLE-AES" | "SAMPLE-AES-CTR" | "SAMPLE-AES-CENC" => KeyMethod::SampleAes,
                other => KeyMethod::Other(other.to_owned()),
            };
            let key_format = attr(&a, "KEYFORMAT").map(str::to_owned);
            let drm_format = key_format
                .as_deref()
                .map(|f| f != "identity")
                .unwrap_or(false);
            match (&method, drm_format) {
                (KeyMethod::None, _) => key = None,
                (KeyMethod::Aes128, false) => {
                    pl.uses_aes128 = true;
                    key = Some(Key {
                        method,
                        uri: match attr(&a, "URI") {
                            Some(u) => Some(resolve(base, u)?),
                            None => None,
                        },
                        iv: attr(&a, "IV").and_then(parse_iv),
                        key_format,
                    });
                }
                _ => {
                    pl.protected = true;
                    key = Some(Key {
                        method,
                        uri: None,
                        iv: None,
                        key_format,
                    });
                }
            }
        } else if let Some(rest) = line.strip_prefix("#EXT-X-MAP:") {
            let a = parse_attributes(rest);
            let uri = attr(&a, "URI")
                .ok_or_else(|| TaskError::new(ErrorKind::ParseError, "EXT-X-MAP without URI"))?;
            let br = attr(&a, "BYTERANGE").map(|r| match r.split_once('@') {
                Some((l, o)) => ByteRange {
                    length: l.parse().unwrap_or(0),
                    offset: o.parse().unwrap_or(0),
                },
                None => ByteRange {
                    length: r.parse().unwrap_or(0),
                    offset: 0,
                },
            });
            pl.init = Some(InitSegment {
                uri: resolve(base, uri)?,
                byte_range: br,
            });
        } else if line.starts_with('#') {
            continue;
        } else {
            let d = duration.take().unwrap_or(pl.target_duration);
            let br = byte_range.take();
            if let Some(r) = &br {
                last_range_end = r.offset + r.length;
            }
            pl.segments.push(Segment {
                index,
                uri: resolve(base, line)?,
                duration: d,
                byte_range: br,
                key: key.clone(),
                sequence: pl.media_sequence + index as u64,
                discontinuity: std::mem::take(&mut discontinuity),
            });
            pl.total_duration += d;
            index += 1;
        }
    }
    if pl.segments.is_empty() {
        return Err(TaskError::new(
            ErrorKind::ParseError,
            "media playlist has no segments",
        ));
    }
    Ok(pl)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_master() {
        let base = Url::parse("https://cdn.example/v/master.m3u8").unwrap();
        let text = "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"English\",DEFAULT=YES,URI=\"audio/en.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH=1280000,RESOLUTION=1280x720,CODECS=\"avc1.4d401f,mp4a.40.2\",FRAME-RATE=29.970,AUDIO=\"aud\"\n720/index.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=2560000,RESOLUTION=1920x1080\nhttps://other.example/1080.m3u8\n";
        match parse(text, &base).unwrap() {
            Playlist::Master(m) => {
                assert_eq!(m.variants.len(), 2);
                assert_eq!(m.variants[0].uri, "https://cdn.example/v/720/index.m3u8");
                assert_eq!(m.variants[0].height, Some(720));
                assert_eq!(
                    m.variants[0].codecs.as_deref(),
                    Some("avc1.4d401f,mp4a.40.2")
                );
                assert_eq!(m.variants[1].uri, "https://other.example/1080.m3u8");
                assert_eq!(
                    m.renditions[0].uri.as_deref(),
                    Some("https://cdn.example/v/audio/en.m3u8")
                );
                assert!(m.renditions[0].default);
            }
            _ => panic!("expected master"),
        }
    }

    #[test]
    fn parses_media_with_keys_and_ranges() {
        let base = Url::parse("https://cdn.example/v/720/index.m3u8").unwrap();
        let text = "#EXTM3U\n#EXT-X-VERSION:4\n#EXT-X-TARGETDURATION:6\n#EXT-X-MEDIA-SEQUENCE:10\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\",IV=0x000102030405060708090a0b0c0d0e0f\n#EXTINF:6.0,\n#EXT-X-BYTERANGE:1000@0\nseg.mp4\n#EXTINF:5.5,\n#EXT-X-BYTERANGE:2000\nseg.mp4\n#EXT-X-KEY:METHOD=NONE\n#EXT-X-DISCONTINUITY\n#EXTINF:4,\nlast.mp4\n#EXT-X-ENDLIST\n";
        match parse(text, &base).unwrap() {
            Playlist::Media(m) => {
                assert_eq!(m.segments.len(), 3);
                assert!(m.end_list);
                assert!(m.uses_aes128 && !m.protected);
                assert_eq!(
                    m.init.as_ref().unwrap().uri,
                    "https://cdn.example/v/720/init.mp4"
                );
                assert_eq!(
                    m.segments[0].byte_range,
                    Some(ByteRange {
                        length: 1000,
                        offset: 0
                    })
                );
                assert_eq!(
                    m.segments[1].byte_range,
                    Some(ByteRange {
                        length: 2000,
                        offset: 1000
                    })
                );
                assert_eq!(m.segments[0].key.as_ref().unwrap().iv.unwrap()[15], 0x0f);
                assert_eq!(m.segments[1].sequence, 11);
                assert!(m.segments[2].key.is_none());
                assert!(m.segments[2].discontinuity);
                assert!((m.total_duration - 15.5).abs() < 1e-9);
            }
            _ => panic!("expected media"),
        }
    }

    #[test]
    fn flags_drm_and_live() {
        let base = Url::parse("https://x/p.m3u8").unwrap();
        let drm = "#EXTM3U\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://x\",KEYFORMAT=\"com.apple.streamingkeydelivery\"\n#EXTINF:4,\na.ts\n";
        match parse(drm, &base).unwrap() {
            Playlist::Media(m) => {
                assert!(m.protected);
                assert!(!m.end_list);
            }
            _ => panic!(),
        }
        assert!(parse("nope", &base).is_err());
    }

    #[test]
    fn attribute_parsing() {
        let a = parse_attributes(r#"BANDWIDTH=1,CODECS="a,b",NAME="x""#);
        assert_eq!(
            a,
            vec![
                ("BANDWIDTH".into(), "1".into()),
                ("CODECS".into(), "a,b".into()),
                ("NAME".into(), "x".into())
            ]
        );
    }
}
