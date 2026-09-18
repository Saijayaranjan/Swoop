//! Metalink parsing: v4 (RFC 5854, `urn:ietf:params:xml:ns:metalink`) and the legacy v3
//! format (`<metalink version="3.0">`). Both describe files with a size, hashes and a ranked
//! list of mirror URLs; the engine treats the URLs as mirrors of identical content and verifies
//! the strongest hash after the download.
//!
//! Only the first `<file>` is downloaded — a multi-file metalink is one task per file in this
//! product, and the services layer is expected to expand it before creating tasks. The parser
//! reports `file_count` so callers can warn.

use osprey_domain::{Checksum, ChecksumAlgorithm, ErrorKind, TaskError};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

/// One mirror from a metalink document, already filtered to `http`/`https`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetalinkUrl {
    pub url: String,
    /// 1 = highest (v4 `priority`; v3 `preference` is converted so callers see one scale).
    pub priority: u32,
    pub location: Option<String>,
}

/// The first file described by a metalink document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetalinkFile {
    /// Sanitised file name (last path component of `<file name>`).
    pub name: String,
    pub size: Option<u64>,
    pub hashes: Vec<Checksum>,
    /// Sorted by priority (best first).
    pub urls: Vec<MetalinkUrl>,
    /// Number of `<file>` entries in the document (>1 means we downloaded only the first).
    pub file_count: usize,
    /// `3` or `4`.
    pub version: u8,
}

impl MetalinkFile {
    /// The strongest verifiable hash (sha512 > sha256 > sha1 > md5).
    pub fn best_hash(&self) -> Option<&Checksum> {
        let rank = |a: ChecksumAlgorithm| match a {
            ChecksumAlgorithm::Sha512 => 4,
            ChecksumAlgorithm::Sha256 | ChecksumAlgorithm::Blake3 => 3,
            ChecksumAlgorithm::Sha1 => 2,
            ChecksumAlgorithm::Md5 => 1,
        };
        self.hashes
            .iter()
            .filter(|h| h.is_well_formed())
            .max_by_key(|h| rank(h.algorithm))
    }

    pub fn mirror_urls(&self) -> Vec<String> {
        self.urls.iter().map(|u| u.url.clone()).collect()
    }
}

/// Maximum document size we are willing to parse (a metalink is a few KB; anything larger is
/// not a metalink).
pub const MAX_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;

/// Parse a Metalink v3 or v4 document and return its first file.
pub fn parse(document: &str) -> Result<MetalinkFile, TaskError> {
    if document.len() > MAX_DOCUMENT_BYTES {
        return Err(parse_err("metalink document too large"));
    }
    let mut reader = Reader::from_str(document);
    reader.config_mut().trim_text(true);
    let mut p = Parser::default();
    loop {
        let ev = reader
            .read_event()
            .map_err(|e| parse_err(format!("invalid XML: {e}")))?;
        match ev {
            Event::Start(e) => p.on_start(&e, false)?,
            Event::Empty(e) => p.on_start(&e, true)?,
            Event::Text(t) => {
                let text = t
                    .unescape()
                    .map_err(|e| parse_err(format!("invalid XML text: {e}")))?;
                p.on_text(text.trim());
            }
            Event::End(_) => p.on_end(),
            Event::Eof => break,
            _ => {}
        }
    }
    p.finish()
}

/// Element-path driven state machine. Only the first `<file>` is materialised.
#[derive(Default)]
struct Parser {
    version: Option<u8>,
    path: Vec<String>,
    file_count: usize,
    first: Option<Builder>,
    /// `type` attribute of a `<hash>` whose text is next.
    pending_hash_type: Option<String>,
    /// (priority, location, protocol allowed) of a `<url>` whose text is next.
    pending_url: Option<(u32, Option<String>, bool)>,
}

#[derive(Default)]
struct Builder {
    name: String,
    size: Option<u64>,
    hashes: Vec<Checksum>,
    urls: Vec<MetalinkUrl>,
}

impl Parser {
    fn version(&self) -> u8 {
        self.version.unwrap_or(4)
    }

    /// v4: metalink/file/…; v3: metalink/files/file/….
    fn file_depth(&self) -> usize {
        if self.version() == 3 {
            3
        } else {
            2
        }
    }

    fn in_first_file(&self) -> bool {
        let d = self.file_depth();
        self.file_count == 1
            && self.path.len() >= d
            && self.path[d - 1] == "file"
            && (self.version() != 3 || self.path.get(1).map(String::as_str) == Some("files"))
    }

    fn on_start(&mut self, e: &BytesStart<'_>, is_empty: bool) -> Result<(), TaskError> {
        let name = local(e.name().local_name().as_ref());
        if self.path.is_empty() {
            if name != "metalink" {
                return Err(parse_err("root element is not <metalink>"));
            }
            self.version = Some(detect_version(e));
        }
        self.path.push(name.clone());
        if name == "file" && self.path.len() == self.file_depth() {
            let at_file =
                self.version() != 3 || self.path.get(1).map(String::as_str) == Some("files");
            if at_file {
                self.file_count += 1;
                if self.first.is_none() {
                    self.first = Some(Builder {
                        name: attr(e, "name").unwrap_or_default(),
                        ..Builder::default()
                    });
                }
            }
        }
        if self.in_first_file() {
            match name.as_str() {
                "hash" => self.pending_hash_type = attr(e, "type"),
                "url" => {
                    let priority = if self.version() == 3 {
                        attr(e, "preference")
                            .and_then(|p| p.parse::<u32>().ok())
                            .map(|p| 101u32.saturating_sub(p.min(100)).max(1))
                            .unwrap_or(50)
                    } else {
                        attr(e, "priority")
                            .and_then(|p| p.parse::<u32>().ok())
                            .unwrap_or(999_999)
                            .max(1)
                    };
                    // v3 carries the protocol in `type`; v4 relies on the URL scheme.
                    let allowed = attr(e, "type")
                        .map(|t| {
                            let t = t.to_ascii_lowercase();
                            t == "http" || t == "https"
                        })
                        .unwrap_or(true);
                    self.pending_url = Some((priority, attr(e, "location"), allowed));
                }
                _ => {}
            }
        }
        if is_empty {
            self.on_end();
        }
        Ok(())
    }

    fn on_text(&mut self, text: &str) {
        if !self.in_first_file() {
            return;
        }
        let size_depth = self.file_depth() + 1;
        let last = self.path.last().cloned().unwrap_or_default();
        let depth = self.path.len();
        let Some(b) = self.first.as_mut() else {
            return;
        };
        match last.as_str() {
            "size" if depth == size_depth => b.size = text.parse::<u64>().ok(),
            "hash" => {
                if let Some(t) = self.pending_hash_type.take() {
                    if let Some(algo) = hash_algorithm(&t) {
                        b.hashes.push(Checksum::new(algo, text));
                    }
                }
            }
            "url" => {
                if let Some((priority, location, allowed)) = self.pending_url.take() {
                    if allowed && is_http(text) {
                        b.urls.push(MetalinkUrl {
                            url: text.to_owned(),
                            priority,
                            location,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    fn on_end(&mut self) {
        self.path.pop();
        self.pending_hash_type = None;
        self.pending_url = None;
    }

    fn finish(self) -> Result<MetalinkFile, TaskError> {
        let version = self.version.ok_or_else(|| parse_err("empty document"))?;
        let b = self
            .first
            .ok_or_else(|| parse_err("metalink has no <file>"))?;
        let mut urls = b.urls;
        urls.sort_by_key(|u| u.priority);
        if urls.is_empty() {
            return Err(parse_err("metalink has no http/https URLs"));
        }
        Ok(MetalinkFile {
            name: file_name(&b.name),
            size: b.size,
            hashes: b.hashes,
            urls,
            file_count: self.file_count,
            version,
        })
    }
}

fn detect_version(root: &BytesStart<'_>) -> u8 {
    let xmlns = root
        .attributes()
        .flatten()
        .find(|a| a.key.as_ref() == b"xmlns")
        .and_then(|a| a.unescape_value().ok().map(|v| v.into_owned()));
    if xmlns.as_deref() == Some("urn:ietf:params:xml:ns:metalink") {
        return 4;
    }
    match attr(root, "version").as_deref() {
        Some(v) if v.starts_with('3') => 3,
        Some(v) if v.starts_with('4') => 4,
        _ => {
            if xmlns.as_deref() == Some("http://www.metalinker.org/") {
                3
            } else {
                4
            }
        }
    }
}

fn local(name: &[u8]) -> String {
    String::from_utf8_lossy(name).to_ascii_lowercase()
}

fn attr(e: &BytesStart<'_>, key: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        if a.key
            .local_name()
            .as_ref()
            .eq_ignore_ascii_case(key.as_bytes())
        {
            a.unescape_value().ok().map(|v| v.trim().to_owned())
        } else {
            None
        }
    })
}

fn hash_algorithm(t: &str) -> Option<ChecksumAlgorithm> {
    match t.to_ascii_lowercase().replace('-', "").as_str() {
        "sha256" => Some(ChecksumAlgorithm::Sha256),
        "sha512" => Some(ChecksumAlgorithm::Sha512),
        "sha1" => Some(ChecksumAlgorithm::Sha1),
        "md5" => Some(ChecksumAlgorithm::Md5),
        "blake3" => Some(ChecksumAlgorithm::Blake3),
        _ => None,
    }
}

fn is_http(url: &str) -> bool {
    let l = url.to_ascii_lowercase();
    l.starts_with("http://") || l.starts_with("https://")
}

/// Metalink names may include directories; single-file downloads keep only the leaf.
fn file_name(raw: &str) -> String {
    let leaf = raw.rsplit(['/', '\\']).next().unwrap_or("").to_owned();
    osprey_runtime::safety::sanitize_filename(&leaf)
}

fn parse_err(msg: impl Into<String>) -> TaskError {
    TaskError::new(ErrorKind::ParseError, msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    const V4: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<metalink xmlns="urn:ietf:params:xml:ns:metalink">
  <published>2009-05-15T12:23:23Z</published>
  <file name="dir/example.ext">
    <size>14471447</size>
    <identity>Example</identity>
    <hash type="sha-256">3d6fece8033d146d8611eab4f032df738c8c1283620fd02a1f2bfec6e27d590d</hash>
    <hash type="md5">ac89bd3c2b1f5d3e6f1a1f4c8e12ab34</hash>
    <url location="de" priority="1">ftp://ftp.example.com/example.ext</url>
    <url location="us" priority="2">https://mirror.example.org/example.ext</url>
    <url priority="1">http://primary.example.com/example.ext</url>
    <metaurl mediatype="torrent">http://example.com/example.ext.torrent</metaurl>
  </file>
  <file name="second.ext"><size>1</size><url>http://x/second</url></file>
</metalink>"#;

    const V3: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<metalink version="3.0" xmlns="http://www.metalinker.org/">
  <files>
    <file name="example.iso">
      <size>1024</size>
      <verification>
        <hash type="sha1">a9993e364706816aba3e25717850c26c9cd0d89d</hash>
        <hash type="sha256">ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad</hash>
        <pieces type="sha1" length="512"><hash piece="0">aa</hash></pieces>
      </verification>
      <resources>
        <url type="ftp" preference="100">ftp://example.com/example.iso</url>
        <url type="http" preference="90">http://a.example.com/example.iso</url>
        <url type="http" preference="100">http://b.example.com/example.iso</url>
      </resources>
    </file>
  </files>
</metalink>"#;

    #[test]
    fn parses_v4() {
        let f = parse(V4).unwrap();
        assert_eq!(f.version, 4);
        assert_eq!(f.name, "example.ext");
        assert_eq!(f.size, Some(14_471_447));
        assert_eq!(f.file_count, 2);
        assert_eq!(
            f.mirror_urls(),
            vec![
                "http://primary.example.com/example.ext",
                "https://mirror.example.org/example.ext"
            ]
        );
        assert_eq!(f.best_hash().unwrap().algorithm, ChecksumAlgorithm::Sha256);
        assert_eq!(f.urls[1].location.as_deref(), Some("us"));
    }

    #[test]
    fn parses_v3() {
        let f = parse(V3).unwrap();
        assert_eq!(f.version, 3);
        assert_eq!(f.name, "example.iso");
        assert_eq!(f.size, Some(1024));
        assert_eq!(f.file_count, 1);
        assert_eq!(
            f.mirror_urls(),
            vec![
                "http://b.example.com/example.iso",
                "http://a.example.com/example.iso"
            ]
        );
        // piece hashes are ignored; whole-file hashes kept
        assert_eq!(f.hashes.len(), 2);
        assert_eq!(f.best_hash().unwrap().algorithm, ChecksumAlgorithm::Sha256);
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(
            parse("<html></html>").unwrap_err().kind,
            ErrorKind::ParseError
        );
        assert_eq!(parse("not xml").unwrap_err().kind, ErrorKind::ParseError);
        let no_urls = r#"<metalink xmlns="urn:ietf:params:xml:ns:metalink"><file name="a"><url>ftp://x/a</url></file></metalink>"#;
        assert_eq!(parse(no_urls).unwrap_err().kind, ErrorKind::ParseError);
    }

    #[test]
    fn sanitises_names_and_handles_empty_elements() {
        let doc = r#"<metalink xmlns="urn:ietf:params:xml:ns:metalink"><file name="../../evil.sh"><identity/><size>5</size><url>http://x/a</url></file></metalink>"#;
        let f = parse(doc).unwrap();
        assert_eq!(f.name, "evil.sh");
        assert_eq!(f.size, Some(5));
    }
}
