//! Media workflows for permitted, non-DRM resources: HLS (M3U8) playlists and direct-media
//! detection. DRM (`SAMPLE-AES`, FairPlay/Widevine key formats) and live streams are refused.

pub mod detect;
pub mod engine;
pub mod m3u8;
pub mod merge;

pub use engine::HlsEngine;
