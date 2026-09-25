//! File classification by extension / MIME and the filter pipeline.

use crate::types::GrabberOptions;
use swoop_domain::rules::glob_match;
use url::Url;

const DOCUMENT: &[&str] = &[
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf", "txt", "md",
    "epub", "mobi", "csv", "json", "xml",
];
const IMAGE: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "heic", "avif", "bmp", "tiff", "tif", "svg", "psd", "raw",
];
const VIDEO: &[&str] = &[
    "mp4", "mkv", "mov", "avi", "webm", "m4v", "wmv", "flv", "ts", "mpg", "mpeg", "3gp",
];
const AUDIO: &[&str] = &[
    "mp3", "m4a", "aac", "flac", "wav", "ogg", "opus", "aiff", "wma",
];
const ARCHIVE: &[&str] = &[
    "zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "zst", "iso", "cab",
];
const SOFTWARE: &[&str] = &[
    "dmg", "pkg", "exe", "msi", "deb", "rpm", "appimage", "apk", "ipa", "jar", "xip",
];
const TORRENT: &[&str] = &["torrent"];
const PLAYLIST: &[&str] = &["m3u8", "m3u", "mpd"];
/// Extensions that are pages, not files.
const PAGE: &[&str] = &[
    "html", "htm", "php", "asp", "aspx", "jsp", "cfm", "cgi", "pl", "shtml", "xhtml",
];

pub fn kind_for_extension(ext: &str) -> &'static str {
    let e = ext.to_ascii_lowercase();
    let e = e.as_str();
    if DOCUMENT.contains(&e) {
        "document"
    } else if IMAGE.contains(&e) {
        "image"
    } else if VIDEO.contains(&e) {
        "video"
    } else if AUDIO.contains(&e) {
        "audio"
    } else if ARCHIVE.contains(&e) {
        "archive"
    } else if SOFTWARE.contains(&e) {
        "software"
    } else if TORRENT.contains(&e) {
        "torrent"
    } else if PLAYLIST.contains(&e) {
        "playlist"
    } else {
        "other"
    }
}

pub fn kind_for_mime(mime: &str) -> Option<&'static str> {
    let m = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    Some(match m.as_str() {
        m if m.starts_with("image/") => "image",
        m if m.starts_with("video/") => "video",
        m if m.starts_with("audio/") => "audio",
        "application/pdf" | "application/epub+zip" | "text/csv" | "text/plain" => "document",
        m if m.contains("zip")
            || m.contains("tar")
            || m.contains("rar")
            || m.contains("7z")
            || m.contains("gzip")
            || m.contains("iso9660") =>
        {
            "archive"
        }
        "application/x-bittorrent" => "torrent",
        "application/vnd.apple.mpegurl" | "application/x-mpegurl" | "application/dash+xml" => {
            "playlist"
        }
        "application/x-apple-diskimage"
        | "application/vnd.microsoft.portable-executable"
        | "application/x-msi"
        | "application/vnd.debian.binary-package"
        | "application/java-archive"
        | "application/vnd.android.package-archive" => "software",
        _ => return None,
    })
}

/// Default interesting extensions when the user did not restrict them.
pub fn default_extensions() -> Vec<String> {
    [
        DOCUMENT, IMAGE, VIDEO, AUDIO, ARCHIVE, SOFTWARE, TORRENT, PLAYLIST,
    ]
    .concat()
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Last path segment's extension, lower-case, or empty.
pub fn extension_of(url: &Url) -> String {
    url.path_segments()
        .and_then(|mut s| s.next_back().map(str::to_owned))
        .and_then(|seg| seg.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()))
        .filter(|e| !e.is_empty() && e.len() <= 8 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or_default()
}

/// Does the URL look like an HTML page (crawl) rather than a file (collect)?
pub fn looks_like_page(url: &Url) -> bool {
    let ext = extension_of(url);
    ext.is_empty() || PAGE.contains(&ext.as_str())
}

/// Filter pipeline for a discovered file URL. Returns the file kind if it passes.
pub fn accept_file(
    opts: &GrabberOptions,
    url: &Url,
    exts: &[String],
    include_regex: Option<&regex::Regex>,
) -> Option<&'static str> {
    let ext = extension_of(url);
    if ext.is_empty() || !exts.iter().any(|e| e == &ext) {
        return None;
    }
    let s = url.as_str();
    if opts
        .exclude_patterns
        .iter()
        .any(|p| glob_match(p, s) || (!p.contains('*') && s.contains(p.as_str())))
    {
        return None;
    }
    if let Some(re) = include_regex {
        if !re.is_match(s) {
            return None;
        }
    }
    Some(kind_for_extension(&ext))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification() {
        assert_eq!(kind_for_extension("PDF"), "document");
        assert_eq!(kind_for_extension("mkv"), "video");
        assert_eq!(kind_for_extension("dmg"), "software");
        assert_eq!(kind_for_mime("image/png"), Some("image"));
        assert_eq!(kind_for_mime("application/zip"), Some("archive"));
        assert_eq!(kind_for_mime("text/html"), None);
        let u = Url::parse("https://x/a/b.tar.gz?x=1").unwrap();
        assert_eq!(extension_of(&u), "gz");
        assert!(looks_like_page(&Url::parse("https://x/a/").unwrap()));
        assert!(looks_like_page(
            &Url::parse("https://x/a/index.php").unwrap()
        ));
        assert!(!looks_like_page(&u));
    }

    #[test]
    fn filters() {
        let opts = GrabberOptions {
            exclude_patterns: vec!["*thumb*".into()],
            ..Default::default()
        };
        let exts = default_extensions();
        assert_eq!(
            accept_file(&opts, &Url::parse("https://x/a.pdf").unwrap(), &exts, None),
            Some("document")
        );
        assert_eq!(
            accept_file(
                &opts,
                &Url::parse("https://x/a_thumb.jpg").unwrap(),
                &exts,
                None
            ),
            None
        );
        let re = regex::Regex::new(r"/reports/").unwrap();
        assert_eq!(
            accept_file(
                &opts,
                &Url::parse("https://x/a.pdf").unwrap(),
                &exts,
                Some(&re)
            ),
            None
        );
        assert_eq!(
            accept_file(
                &opts,
                &Url::parse("https://x/reports/a.pdf").unwrap(),
                &exts,
                Some(&re)
            ),
            Some("document")
        );
    }
}
