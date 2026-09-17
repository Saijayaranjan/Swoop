//! Filename resolution from URLs and HTTP headers (RFC 6266 `Content-Disposition`, including
//! RFC 5987 `filename*=UTF-8''…`), with MIME-based extension fallback.

use crate::safety::sanitize_filename;
use percent_encoding::percent_decode_str;

/// Parse a `Content-Disposition` header value into a filename, preferring `filename*`.
pub fn from_content_disposition(value: &str) -> Option<String> {
    let mut plain: Option<String> = None;
    let mut ext: Option<String> = None;
    for part in split_params(value) {
        let part = part.trim();
        let Some((k, v)) = part.split_once('=') else { continue };
        let key = k.trim().to_ascii_lowercase();
        let v = v.trim();
        if key == "filename*" {
            // charset'lang'value
            let mut it = v.splitn(3, '\'');
            let charset = it.next().unwrap_or("").to_ascii_lowercase();
            let _lang = it.next();
            if let Some(enc) = it.next() {
                let decoded = percent_decode_str(enc).decode_utf8_lossy().to_string();
                if charset == "utf-8" || charset == "utf8" || charset.is_empty() {
                    ext = Some(decoded);
                } else if charset == "iso-8859-1" || charset == "latin1" {
                    let bytes: Vec<u8> = percent_decode_str(enc).collect();
                    ext = Some(bytes.iter().map(|&b| b as char).collect());
                } else {
                    ext = Some(decoded);
                }
            }
        } else if key == "filename" {
            let unq = unquote(v);
            // Some servers percent-encode plain filename; decode only if it looks encoded.
            let decoded = if unq.contains('%') { percent_decode_str(&unq).decode_utf8_lossy().to_string() } else { unq };
            plain = Some(decoded);
        }
    }
    let raw = ext.or(plain)?;
    // A path may be smuggled in; keep only the last component.
    let last = raw.rsplit(['/', '\\']).next().unwrap_or(&raw).to_owned();
    let name = sanitize_filename(&last);
    if name == "download" && !last.trim().is_empty() && last.trim() != "download" {
        // sanitised to the fallback; treat as absent so URL-derived name is used
        return None;
    }
    Some(name)
}

fn split_params(value: &str) -> Vec<String> {
    // split on ';' but not inside quotes
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    for c in value.chars() {
        match c {
            '"' => {
                in_q = !in_q;
                cur.push(c);
            }
            ';' if !in_q => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        v[1..v.len() - 1].replace("\\\"", "\"")
    } else {
        v.to_owned()
    }
}

/// Filename from the last path segment of a URL (percent-decoded), ignoring query strings.
pub fn from_url(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let seg = parsed.path_segments()?.filter(|s| !s.is_empty()).next_back()?;
    let decoded = percent_decode_str(seg).decode_utf8_lossy().to_string();
    let name = sanitize_filename(&decoded);
    if name == "download" && decoded.trim() != "download" {
        return None;
    }
    Some(name)
}

/// Best-effort name: header → URL → host-based fallback, adding an extension from MIME when
/// the name has none.
pub fn resolve(url: &str, content_disposition: Option<&str>, content_type: Option<&str>) -> String {
    let mut name = content_disposition
        .and_then(from_content_disposition)
        .or_else(|| from_url(url))
        .unwrap_or_else(|| {
            let host = url::Url::parse(url).ok().and_then(|u| u.host_str().map(|h| h.to_owned())).unwrap_or_else(|| "download".into());
            format!("{}-download", host.replace('.', "-"))
        });
    let has_ext = std::path::Path::new(&name).extension().map(|e| !e.is_empty() && e.len() <= 8).unwrap_or(false);
    if !has_ext {
        if let Some(ct) = content_type {
            let mime = ct.split(';').next().unwrap_or("").trim();
            if let Some(ext) = extension_for_mime(mime) {
                name = format!("{name}.{ext}");
            }
        }
    }
    name
}

/// Preferred extension for a MIME type (only for common, unambiguous types).
pub fn extension_for_mime(mime: &str) -> Option<&'static str> {
    Some(match mime.to_ascii_lowercase().as_str() {
        "application/pdf" => "pdf",
        "application/zip" | "application/x-zip-compressed" => "zip",
        "application/gzip" | "application/x-gzip" => "gz",
        "application/x-tar" => "tar",
        "application/x-7z-compressed" => "7z",
        "application/x-rar-compressed" | "application/vnd.rar" => "rar",
        "application/x-apple-diskimage" => "dmg",
        "application/x-iso9660-image" => "iso",
        "application/x-bittorrent" => "torrent",
        "application/json" => "json",
        "application/xml" | "text/xml" => "xml",
        "text/plain" => "txt",
        "text/html" => "html",
        "text/csv" => "csv",
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "image/heic" => "heic",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/x-matroska" => "mkv",
        "video/quicktime" => "mov",
        "video/mp2t" => "ts",
        "audio/mpeg" => "mp3",
        "audio/mp4" | "audio/x-m4a" => "m4a",
        "audio/flac" | "audio/x-flac" => "flac",
        "audio/ogg" => "ogg",
        "audio/wav" | "audio/x-wav" => "wav",
        "application/vnd.apple.mpegurl" | "application/x-mpegurl" => "m3u8",
        "application/epub+zip" => "epub",
        "application/vnd.android.package-archive" => "apk",
        "application/x-msdownload" | "application/vnd.microsoft.portable-executable" => "exe",
        "application/x-debian-package" | "application/vnd.debian.binary-package" => "deb",
        "application/x-rpm" => "rpm",
        _ => return None,
    })
}

/// Guess a MIME type from a filename.
pub fn mime_for_name(name: &str) -> Option<String> {
    mime_guess::from_path(name).first().map(|m| m.essence_str().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_disposition_variants() {
        assert_eq!(from_content_disposition(r#"attachment; filename="report.pdf""#).as_deref(), Some("report.pdf"));
        assert_eq!(from_content_disposition("attachment; filename*=UTF-8''r%C3%A9sum%C3%A9.pdf").as_deref(), Some("résumé.pdf"));
        assert_eq!(
            from_content_disposition(r#"attachment; filename="fallback.txt"; filename*=UTF-8''better.txt"#).as_deref(),
            Some("better.txt")
        );
        assert_eq!(from_content_disposition(r#"attachment; filename="../../evil.sh""#).as_deref(), Some("evil.sh"));
        assert_eq!(from_content_disposition("inline").is_none(), true);
        assert_eq!(from_content_disposition(r#"attachment; filename="a; b.txt""#).as_deref(), Some("a; b.txt"));
    }

    #[test]
    fn url_names() {
        assert_eq!(from_url("https://x.com/a/b/c%20d.zip?token=1").as_deref(), Some("c d.zip"));
        assert_eq!(from_url("https://x.com/").is_none(), true);
        assert_eq!(resolve("https://x.com/", None, Some("application/pdf")), "x-com-download.pdf");
        assert_eq!(resolve("https://x.com/file", None, Some("video/mp4; charset=binary")), "file.mp4");
    }
}
