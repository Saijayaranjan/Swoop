//! Direct-media classification and HLS description for the add dialog / browser extension.

use crate::m3u8::{self, Playlist};
use std::sync::Arc;
use swoop_domain::media::{DetectedMedia, MediaInfo, MediaKind, MediaVariant};
use swoop_domain::{ErrorKind, TaskError};
use swoop_runtime::net::ClientFactory;
use url::Url;

const VIDEO_EXT: &[&str] = &[
    "mp4", "m4v", "mkv", "webm", "mov", "avi", "wmv", "flv", "ts", "mpg", "mpeg", "3gp",
];
const AUDIO_EXT: &[&str] = &[
    "mp3", "m4a", "aac", "flac", "wav", "ogg", "opus", "aiff", "wma", "alac",
];
const IMAGE_EXT: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "heic", "avif", "bmp", "tiff", "svg",
];

/// Classify by MIME first, then by URL extension.
pub fn classify(url: &str, mime: Option<&str>) -> MediaKind {
    if let Some(m) = mime {
        let m = m
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if m == "application/vnd.apple.mpegurl"
            || m == "application/x-mpegurl"
            || m == "audio/mpegurl"
            || m == "audio/x-mpegurl"
        {
            return MediaKind::HlsPlaylist;
        }
        if m == "application/dash+xml" {
            return MediaKind::DashManifest;
        }
        if m.starts_with("video/") {
            return MediaKind::Video;
        }
        if m.starts_with("audio/") {
            return MediaKind::Audio;
        }
        if m.starts_with("image/") {
            return MediaKind::Image;
        }
    }
    let ext = Url::parse(url)
        .ok()
        .and_then(|u| {
            u.path_segments()
                .and_then(|mut s| s.next_back().map(str::to_owned))
        })
        .and_then(|seg| seg.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()))
        .unwrap_or_default();
    match ext.as_str() {
        "m3u8" | "m3u" => MediaKind::HlsPlaylist,
        "mpd" => MediaKind::DashManifest,
        e if VIDEO_EXT.contains(&e) => MediaKind::Video,
        e if AUDIO_EXT.contains(&e) => MediaKind::Audio,
        e if IMAGE_EXT.contains(&e) => MediaKind::Image,
        _ => MediaKind::Unknown,
    }
}

/// Human label for a variant ("1080p · 2.5 Mb/s · avc1").
pub fn variant_label(v: &m3u8::Variant) -> String {
    let mut parts = Vec::new();
    if let Some(h) = v.height {
        parts.push(format!("{h}p"));
    } else if let Some(n) = &v.name {
        parts.push(n.clone());
    }
    if let Some(b) = v.average_bandwidth.or(v.bandwidth) {
        parts.push(format!("{:.1} Mb/s", b as f64 / 1_000_000.0));
    }
    if let Some(c) = &v.codecs {
        let short = c
            .split(',')
            .next()
            .unwrap_or("")
            .split('.')
            .next()
            .unwrap_or("");
        if !short.is_empty() {
            parts.push(short.to_owned());
        }
    }
    if parts.is_empty() {
        "default".to_owned()
    } else {
        parts.join(" · ")
    }
}

/// Fetch a playlist (bounded to 4 MiB).
pub async fn fetch_playlist(
    client: &reqwest::Client,
    url: &Url,
) -> Result<(String, Url), TaskError> {
    let resp = client.get(url.clone()).send().await.map_err(|e| {
        TaskError::new(ErrorKind::ConnectionReset, format!("playlist fetch: {e}"))
            .with_source(url.to_string())
    })?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(TaskError::from_http_status(status, url.as_str()));
    }
    let final_url = resp.url().clone();
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| TaskError::new(ErrorKind::Truncated, format!("playlist body: {e}")))?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(TaskError::new(
            ErrorKind::ParseError,
            "playlist larger than 4 MiB",
        ));
    }
    Ok((String::from_utf8_lossy(&bytes).into_owned(), final_url))
}

/// Describe an HLS URL: variants with resolution/bandwidth/estimated size, protection flag.
pub async fn describe_hls(url: &str, clients: Arc<ClientFactory>) -> Result<MediaInfo, TaskError> {
    let parsed =
        Url::parse(url).map_err(|e| TaskError::new(ErrorKind::InvalidUrl, e.to_string()))?;
    let client = clients.default_client()?;
    let (text, base) = fetch_playlist(&client, &parsed).await?;
    match m3u8::parse(&text, &base)? {
        Playlist::Master(master) => {
            // Duration from the first variant's media playlist (best effort, bounded).
            let mut duration: Option<f64> = None;
            let mut protected = false;
            if let Some(first) = master.variants.first() {
                if let Ok(u) = Url::parse(&first.uri) {
                    if let Ok((t, b)) = fetch_playlist(&client, &u).await {
                        if let Ok(Playlist::Media(m)) = m3u8::parse(&t, &b) {
                            duration = Some(m.total_duration);
                            protected = m.protected;
                        }
                    }
                }
            }
            let mut variants: Vec<MediaVariant> = master
                .variants
                .iter()
                .map(|v| MediaVariant {
                    id: v.uri.clone(),
                    label: variant_label(v),
                    url: v.uri.clone(),
                    width: v.width,
                    height: v.height,
                    bandwidth: v.average_bandwidth.or(v.bandwidth),
                    codecs: v.codecs.clone(),
                    frame_rate: v.frame_rate,
                    estimated_size: match (duration, v.average_bandwidth.or(v.bandwidth)) {
                        (Some(d), Some(b)) => Some((d * b as f64 / 8.0) as u64),
                        _ => None,
                    },
                    audio_only: v.height.is_none()
                        && v.codecs
                            .as_deref()
                            .map(|c| c.starts_with("mp4a"))
                            .unwrap_or(false),
                    container: None,
                })
                .collect();
            variants.sort_by_key(|v| std::cmp::Reverse(v.bandwidth.unwrap_or(0)));
            for r in master
                .renditions
                .iter()
                .filter(|r| r.kind.eq_ignore_ascii_case("AUDIO"))
            {
                if let Some(u) = &r.uri {
                    variants.push(MediaVariant {
                        id: u.clone(),
                        label: format!(
                            "Audio · {}",
                            if r.name.is_empty() {
                                r.group_id.clone()
                            } else {
                                r.name.clone()
                            }
                        ),
                        url: u.clone(),
                        width: None,
                        height: None,
                        bandwidth: None,
                        codecs: None,
                        frame_rate: None,
                        estimated_size: None,
                        audio_only: true,
                        container: None,
                    });
                }
            }
            Ok(MediaInfo {
                kind: MediaKind::HlsPlaylist,
                title: None,
                format: Some("hls".into()),
                duration_seconds: duration,
                variants,
                selected_variant: None,
                segment_count: None,
                segments_done: 0,
                protected,
                page_url: None,
            })
        }
        Playlist::Media(media) => Ok(MediaInfo {
            kind: MediaKind::HlsPlaylist,
            title: None,
            format: Some(
                if media.init.is_some() {
                    "hls-fmp4"
                } else {
                    "hls-ts"
                }
                .into(),
            ),
            duration_seconds: Some(media.total_duration),
            variants: vec![MediaVariant {
                id: base.to_string(),
                label: "default".into(),
                url: base.to_string(),
                width: None,
                height: None,
                bandwidth: None,
                codecs: None,
                frame_rate: None,
                estimated_size: None,
                audio_only: false,
                container: Some(if media.init.is_some() { "mp4" } else { "ts" }.into()),
            }],
            selected_variant: None,
            segment_count: Some(media.segments.len() as u32),
            segments_done: 0,
            protected: media.protected || !media.end_list,
            page_url: None,
        }),
    }
}

/// Detect what a URL is (HEAD for direct media, playlist fetch for HLS).
pub async fn detect(
    url: &str,
    page_url: Option<String>,
    clients: Arc<ClientFactory>,
) -> Result<DetectedMedia, TaskError> {
    let parsed =
        Url::parse(url).map_err(|e| TaskError::new(ErrorKind::InvalidUrl, e.to_string()))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(TaskError::new(
            ErrorKind::UnsupportedScheme,
            parsed.scheme().to_owned(),
        ));
    }
    let client = clients.default_client()?;
    let head = client.head(parsed.clone()).send().await.ok();
    let (mime, size) = match &head {
        Some(r) if r.status().is_success() => (
            r.headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.split(';').next().unwrap_or("").trim().to_owned()),
            r.headers()
                .get(reqwest::header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse().ok()),
        ),
        _ => (None, None),
    };
    let kind = classify(url, mime.as_deref());
    let mut out = DetectedMedia {
        url: url.to_owned(),
        kind,
        title: None,
        mime: mime.clone(),
        size,
        page_url,
        variants: Vec::new(),
        protected: false,
    };
    if kind == MediaKind::HlsPlaylist {
        match describe_hls(url, clients).await {
            Ok(info) => {
                out.variants = info.variants;
                out.protected = info.protected;
            }
            Err(e) => {
                return Err(e);
            }
        }
    } else if kind == MediaKind::DashManifest {
        out.protected = true; // not downloadable in this version
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies() {
        assert_eq!(
            classify("https://x/a.m3u8?x=1", None),
            MediaKind::HlsPlaylist
        );
        assert_eq!(
            classify("https://x/a", Some("video/mp4; codecs=avc1")),
            MediaKind::Video
        );
        assert_eq!(classify("https://x/a.mp3", None), MediaKind::Audio);
        assert_eq!(classify("https://x/a.png", None), MediaKind::Image);
        assert_eq!(classify("https://x/a.mpd", None), MediaKind::DashManifest);
        assert_eq!(classify("https://x/a.bin", None), MediaKind::Unknown);
    }

    #[test]
    fn labels() {
        let v = m3u8::Variant {
            uri: "u".into(),
            bandwidth: Some(2_500_000),
            average_bandwidth: None,
            width: Some(1920),
            height: Some(1080),
            codecs: Some("avc1.4d,mp4a".into()),
            frame_rate: None,
            audio_group: None,
            name: None,
        };
        assert_eq!(variant_label(&v), "1080p · 2.5 Mb/s · avc1");
    }
}
