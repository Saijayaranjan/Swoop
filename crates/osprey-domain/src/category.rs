//! Categories classify tasks (Documents, Images, …) and drive default destinations.

use crate::{CategoryId, Millis};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Category {
    pub id: CategoryId,
    pub name: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub color: Option<String>,
    /// Lower-case extensions without the dot.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// MIME prefixes such as `video/`.
    #[serde(default)]
    pub mime_prefixes: Vec<String>,
    /// Subdirectory (relative) or absolute directory for files in this category.
    #[serde(default)]
    pub directory: Option<PathBuf>,
    #[serde(default)]
    pub builtin: bool,
    #[serde(default)]
    pub position: i32,
    pub created_at: Millis,
    pub updated_at: Millis,
}

impl Category {
    pub fn new(name: impl Into<String>) -> Self {
        let now = Millis::now();
        Self {
            id: CategoryId::new(),
            name: name.into(),
            icon: "folder".into(),
            color: None,
            extensions: Vec::new(),
            mime_prefixes: Vec::new(),
            directory: None,
            builtin: false,
            position: 0,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn matches_extension(&self, ext: &str) -> bool {
        let ext = ext.trim_start_matches('.').to_ascii_lowercase();
        self.extensions.iter().any(|e| *e == ext)
    }

    pub fn matches_mime(&self, mime: &str) -> bool {
        let mime = mime.to_ascii_lowercase();
        self.mime_prefixes.iter().any(|p| mime.starts_with(p.as_str()))
    }

    pub fn builtin_defaults() -> Vec<Category> {
        let mk = |id: &str, name: &str, icon: &str, exts: &[&str], mimes: &[&str], dir: &str, pos: i32| {
            let mut c = Category::new(name);
            c.id = CategoryId(id.to_owned());
            c.icon = icon.to_owned();
            c.extensions = exts.iter().map(|s| s.to_string()).collect();
            c.mime_prefixes = mimes.iter().map(|s| s.to_string()).collect();
            c.directory = Some(PathBuf::from(dir));
            c.builtin = true;
            c.position = pos;
            c
        };
        vec![
            mk("cat-documents", "Documents", "doc.text",
               &["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "txt", "rtf", "md", "epub", "mobi", "csv", "pages", "numbers", "key"],
               &["application/pdf", "text/", "application/msword", "application/vnd.openxmlformats", "application/epub"], "Documents", 0),
            mk("cat-images", "Images", "photo",
               &["jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "bmp", "tiff", "tif", "svg", "avif", "raw", "cr2", "nef", "psd", "ai"],
               &["image/"], "Images", 1),
            mk("cat-videos", "Videos", "film",
               &["mp4", "mkv", "mov", "avi", "webm", "m4v", "wmv", "flv", "ts", "m3u8", "mpg", "mpeg", "3gp"],
               &["video/", "application/vnd.apple.mpegurl", "application/x-mpegurl"], "Videos", 2),
            mk("cat-audio", "Audio", "music.note",
               &["mp3", "m4a", "aac", "flac", "wav", "ogg", "opus", "aiff", "wma", "alac"],
               &["audio/"], "Audio", 3),
            mk("cat-archives", "Archives", "archivebox",
               &["zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "zst", "iso", "cab"],
               &["application/zip", "application/x-rar", "application/x-7z", "application/x-tar", "application/gzip", "application/x-iso9660-image"], "Archives", 4),
            mk("cat-software", "Software", "app.badge",
               &["dmg", "pkg", "app", "exe", "msi", "deb", "rpm", "appimage", "apk", "ipa", "jar", "xip"],
               &["application/x-apple-diskimage", "application/vnd.microsoft.portable-executable", "application/x-msi", "application/vnd.debian.binary-package", "application/java-archive"], "Software", 5),
            mk("cat-torrents", "Torrents", "network",
               &["torrent"], &["application/x-bittorrent"], "Torrents", 6),
            mk("cat-other", "Other", "shippingbox", &[], &[], "", 7),
        ]
    }
}
