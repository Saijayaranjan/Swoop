//! Media detection results for permitted, non-DRM resources.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Video,
    Audio,
    Image,
    HlsPlaylist,
    /// DASH manifests are detected for display but not downloaded in this version.
    DashManifest,
    Unknown,
}

/// One selectable rendition of an HLS master playlist or a direct-media alternative.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MediaVariant {
    /// Stable identifier (playlist URL or a synthetic key).
    pub id: String,
    pub label: String,
    pub url: String,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub bandwidth: Option<u64>,
    #[serde(default)]
    pub codecs: Option<String>,
    #[serde(default)]
    pub frame_rate: Option<f32>,
    /// Estimated size in bytes if duration and bandwidth are known.
    #[serde(default)]
    pub estimated_size: Option<u64>,
    #[serde(default)]
    pub audio_only: bool,
    #[serde(default)]
    pub container: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MediaInfo {
    pub kind: MediaKind,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub duration_seconds: Option<f64>,
    #[serde(default)]
    pub variants: Vec<MediaVariant>,
    #[serde(default)]
    pub selected_variant: Option<String>,
    /// Number of HLS segments once the media playlist is parsed.
    #[serde(default)]
    pub segment_count: Option<u32>,
    #[serde(default)]
    pub segments_done: u32,
    /// The playlist declared encryption we do not handle (DRM / SAMPLE-AES). We refuse such media.
    #[serde(default)]
    pub protected: bool,
    /// Page the media was detected on.
    #[serde(default)]
    pub page_url: Option<String>,
}

/// A media resource detected by the browser extension or the site grabber.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetectedMedia {
    pub url: String,
    pub kind: MediaKind,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub mime: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub page_url: Option<String>,
    #[serde(default)]
    pub variants: Vec<MediaVariant>,
    #[serde(default)]
    pub protected: bool,
}
