//! Archive intelligence: list entries and extract selectively, safely (no zip-slip, no
//! symlinks, no absolute paths), for ZIP, TAR, TAR.GZ/TGZ and GZ. Formats we cannot read
//! (RAR, 7z) are reported by magic bytes with `supports_selective_extraction = false`.

use osprey_domain::{ErrorKind, Millis, TaskError};
use osprey_runtime::safety;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArchiveEntry {
    pub path: String,
    pub size: u64,
    pub compressed_size: Option<u64>,
    pub is_dir: bool,
    pub modified: Option<Millis>,
    pub crc32: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ArchiveListing {
    pub format: String,
    pub entries: Vec<ArchiveEntry>,
    pub truncated: bool,
    pub intact: Option<bool>,
    pub supports_selective_extraction: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Zip,
    Tar,
    TarGz,
    Gz,
    Rar,
    SevenZ,
    Unknown,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::Zip => "zip",
            Format::Tar => "tar",
            Format::TarGz => "tar.gz",
            Format::Gz => "gz",
            Format::Rar => "rar",
            Format::SevenZ => "7z",
            Format::Unknown => "unknown",
        }
    }
}

/// Detect by magic bytes, falling back to the extension for TAR (which has no magic at offset 0).
pub fn detect(path: &Path) -> std::io::Result<Format> {
    let mut f = std::fs::File::open(path)?;
    let mut head = [0u8; 8];
    let n = f.read(&mut head)?;
    let head = &head[..n];
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.to_ascii_lowercase())
        .unwrap_or_default();
    Ok(
        if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
            Format::Zip
        } else if head.starts_with(b"\x1f\x8b") {
            if name.ends_with(".tar.gz") || ext == "tgz" {
                Format::TarGz
            } else {
                Format::Gz
            }
        } else if head.starts_with(b"Rar!") {
            Format::Rar
        } else if head.starts_with(b"7z\xbc\xaf\x27\x1c") {
            Format::SevenZ
        } else if ext == "tar" || tar_magic(path) {
            Format::Tar
        } else {
            Format::Unknown
        },
    )
}

fn tar_magic(path: &Path) -> bool {
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut buf = [0u8; 265];
    if f.read_exact(&mut buf).is_err() {
        return false;
    }
    &buf[257..262] == b"ustar"
}

pub async fn list(path: &Path, limit: usize) -> Result<ArchiveListing, TaskError> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || list_blocking(&path, limit))
        .await
        .map_err(|e| TaskError::internal(e.to_string()))?
}

fn list_blocking(path: &Path, limit: usize) -> Result<ArchiveListing, TaskError> {
    let format = detect(path).map_err(|e| TaskError::from_io(&e, "open archive"))?;
    let limit = limit.clamp(1, 100_000);
    match format {
        Format::Zip => {
            let f = std::fs::File::open(path).map_err(|e| TaskError::from_io(&e, "open"))?;
            let mut z = match zip::ZipArchive::new(f) {
                Ok(z) => z,
                Err(e) => {
                    return Ok(ArchiveListing {
                        format: "zip".into(),
                        entries: vec![],
                        truncated: false,
                        intact: Some(false),
                        supports_selective_extraction: false,
                    })
                    .map(|mut l| {
                        tracing::debug!("zip open failed: {e}");
                        l.intact = Some(false);
                        l
                    })
                }
            };
            let mut entries = Vec::new();
            let total = z.len();
            for i in 0..total.min(limit) {
                let e = z
                    .by_index_raw(i)
                    .map_err(|e| TaskError::new(ErrorKind::ParseError, e.to_string()))?;
                let modified = e.last_modified().and_then(|d| {
                    let dt: Option<time_lite::Dt> = time_lite::from_zip(
                        d.year() as i32,
                        d.month() as u32,
                        d.day() as u32,
                        d.hour() as u32,
                        d.minute() as u32,
                        d.second() as u32,
                    );
                    dt.map(|d| Millis(d.0 * 1000))
                });
                entries.push(ArchiveEntry {
                    path: e.name().to_owned(),
                    size: e.size(),
                    compressed_size: Some(e.compressed_size()),
                    is_dir: e.is_dir(),
                    modified,
                    crc32: Some(e.crc32()),
                });
            }
            Ok(ArchiveListing {
                format: "zip".into(),
                entries,
                truncated: total > limit,
                intact: Some(true),
                supports_selective_extraction: true,
            })
        }
        Format::Tar | Format::TarGz => {
            let f = std::fs::File::open(path).map_err(|e| TaskError::from_io(&e, "open"))?;
            let reader: Box<dyn Read> = if format == Format::TarGz {
                Box::new(flate2::read::GzDecoder::new(f))
            } else {
                Box::new(f)
            };
            let mut a = tar::Archive::new(reader);
            let mut entries = Vec::new();
            let mut truncated = false;
            let mut intact = true;
            match a.entries() {
                Ok(iter) => {
                    for e in iter {
                        match e {
                            Ok(e) => {
                                if entries.len() >= limit {
                                    truncated = true;
                                    break;
                                }
                                let header = e.header();
                                let p = e
                                    .path()
                                    .map(|p| p.to_string_lossy().into_owned())
                                    .unwrap_or_default();
                                entries.push(ArchiveEntry {
                                    path: p,
                                    size: header.size().unwrap_or(0),
                                    compressed_size: None,
                                    is_dir: header.entry_type().is_dir(),
                                    modified: header.mtime().ok().map(|m| Millis(m as i64 * 1000)),
                                    crc32: None,
                                });
                            }
                            Err(_) => {
                                intact = false;
                                break;
                            }
                        }
                    }
                }
                Err(_) => intact = false,
            }
            Ok(ArchiveListing {
                format: format.name().into(),
                entries,
                truncated,
                intact: Some(intact),
                supports_selective_extraction: true,
            })
        }
        Format::Gz => {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("file")
                .to_owned();
            let f = std::fs::File::open(path).map_err(|e| TaskError::from_io(&e, "open"))?;
            // ISIZE trailer holds the uncompressed size mod 2^32
            let len = f.metadata().map(|m| m.len()).unwrap_or(0);
            let mut size = 0u64;
            if len >= 8 {
                use std::io::Seek;
                let mut f = f;
                let mut tail = [0u8; 4];
                if f.seek(std::io::SeekFrom::End(-4)).is_ok() && f.read_exact(&mut tail).is_ok() {
                    size = u32::from_le_bytes(tail) as u64;
                }
            }
            Ok(ArchiveListing {
                format: "gz".into(),
                entries: vec![ArchiveEntry {
                    path: name,
                    size,
                    compressed_size: Some(len),
                    is_dir: false,
                    modified: None,
                    crc32: None,
                }],
                truncated: false,
                intact: None,
                supports_selective_extraction: false,
            })
        }
        other => Ok(ArchiveListing {
            format: other.name().into(),
            entries: vec![],
            truncated: false,
            intact: None,
            supports_selective_extraction: false,
        }),
    }
}

/// Extract `entries` (all when `None`) into `destination`, refusing traversal, absolute paths
/// and symlinks. Returns the number of files written.
pub async fn extract(
    path: &Path,
    entries: Option<&[String]>,
    destination: &Path,
) -> Result<u32, TaskError> {
    let path = path.to_path_buf();
    let dest = destination.to_path_buf();
    let wanted: Option<Vec<String>> = entries.map(|e| e.to_vec());
    tokio::task::spawn_blocking(move || extract_blocking(&path, wanted.as_deref(), &dest))
        .await
        .map_err(|e| TaskError::internal(e.to_string()))?
}

fn wanted_contains(wanted: Option<&[String]>, name: &str) -> bool {
    match wanted {
        None => true,
        Some(list) => list
            .iter()
            .any(|w| w == name || (w.ends_with('/') && name.starts_with(w.as_str()))),
    }
}

fn target_for(dest: &Path, name: &str) -> Result<PathBuf, TaskError> {
    let rel = safety::sanitize_relative_path(name)?;
    let full = dest.join(rel);
    safety::ensure_within(dest, &full)?;
    Ok(full)
}

fn extract_blocking(path: &Path, wanted: Option<&[String]>, dest: &Path) -> Result<u32, TaskError> {
    safety::validate_destination_dir(dest)?;
    std::fs::create_dir_all(dest).map_err(|e| TaskError::from_io(&e, "create destination"))?;
    let format = detect(path).map_err(|e| TaskError::from_io(&e, "open archive"))?;
    let mut count = 0u32;
    match format {
        Format::Zip => {
            let f = std::fs::File::open(path).map_err(|e| TaskError::from_io(&e, "open"))?;
            let mut z = zip::ZipArchive::new(f)
                .map_err(|e| TaskError::new(ErrorKind::ParseError, e.to_string()))?;
            for i in 0..z.len() {
                let mut e = z
                    .by_index(i)
                    .map_err(|e| TaskError::new(ErrorKind::ParseError, e.to_string()))?;
                let name = e.name().to_owned();
                if !wanted_contains(wanted, &name) {
                    continue;
                }
                if e.is_symlink() {
                    continue;
                }
                let target = match target_for(dest, &name) {
                    Ok(t) => t,
                    Err(err) => {
                        tracing::warn!(entry = %name, "skipping unsafe archive entry: {}", err.message);
                        continue;
                    }
                };
                if e.is_dir() {
                    std::fs::create_dir_all(&target)
                        .map_err(|e| TaskError::from_io(&e, "mkdir"))?;
                    continue;
                }
                if let Some(p) = target.parent() {
                    std::fs::create_dir_all(p).map_err(|e| TaskError::from_io(&e, "mkdir"))?;
                }
                if safety::symlink_escapes(dest, &target) {
                    return Err(TaskError::new(
                        ErrorKind::PathTraversal,
                        "existing symlink at target",
                    ));
                }
                let mut out =
                    std::fs::File::create(&target).map_err(|e| TaskError::from_io(&e, "create"))?;
                std::io::copy(&mut e, &mut out).map_err(|e| TaskError::from_io(&e, "write"))?;
                count += 1;
            }
        }
        Format::Tar | Format::TarGz => {
            let f = std::fs::File::open(path).map_err(|e| TaskError::from_io(&e, "open"))?;
            let reader: Box<dyn Read> = if format == Format::TarGz {
                Box::new(flate2::read::GzDecoder::new(f))
            } else {
                Box::new(f)
            };
            let mut a = tar::Archive::new(reader);
            for e in a
                .entries()
                .map_err(|e| TaskError::new(ErrorKind::ParseError, e.to_string()))?
            {
                let mut e = e.map_err(|e| TaskError::new(ErrorKind::ParseError, e.to_string()))?;
                let name = e
                    .path()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if !wanted_contains(wanted, &name) {
                    continue;
                }
                let kind = e.header().entry_type();
                if kind.is_symlink() || kind.is_hard_link() {
                    continue;
                }
                let target = match target_for(dest, &name) {
                    Ok(t) => t,
                    Err(err) => {
                        tracing::warn!(entry = %name, "skipping unsafe archive entry: {}", err.message);
                        continue;
                    }
                };
                if kind.is_dir() {
                    std::fs::create_dir_all(&target)
                        .map_err(|e| TaskError::from_io(&e, "mkdir"))?;
                    continue;
                }
                if !kind.is_file() {
                    continue;
                }
                if let Some(p) = target.parent() {
                    std::fs::create_dir_all(p).map_err(|e| TaskError::from_io(&e, "mkdir"))?;
                }
                let mut out =
                    std::fs::File::create(&target).map_err(|e| TaskError::from_io(&e, "create"))?;
                std::io::copy(&mut e, &mut out).map_err(|e| TaskError::from_io(&e, "write"))?;
                count += 1;
            }
        }
        Format::Gz => {
            let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
            let target = target_for(dest, name)?;
            let f = std::fs::File::open(path).map_err(|e| TaskError::from_io(&e, "open"))?;
            let mut d = flate2::read::GzDecoder::new(f);
            let mut out =
                std::fs::File::create(&target).map_err(|e| TaskError::from_io(&e, "create"))?;
            std::io::copy(&mut d, &mut out).map_err(|e| TaskError::from_io(&e, "write"))?;
            count = 1;
        }
        other => {
            return Err(TaskError::new(
                ErrorKind::UnsupportedScheme,
                format!("extraction of {} archives is not supported", other.name()),
            ))
        }
    }
    Ok(count)
}

mod time_lite {
    pub struct Dt(pub i64);
    pub fn from_zip(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> Option<Dt> {
        if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
            return None;
        }
        let y2 = if m <= 2 { y - 1 } else { y };
        let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
        let yoe = (y2 - era * 400) as u32;
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era as i64 * 146_097 + doe as i64 - 719_468;
        Some(Dt(days * 86_400
            + h as i64 * 3600
            + mi as i64 * 60
            + s as i64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_zip(dir: &Path) -> PathBuf {
        let p = dir.join("a.zip");
        let f = std::fs::File::create(&p).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default();
        z.add_directory("docs/", opts).unwrap();
        z.start_file("docs/readme.txt", opts).unwrap();
        z.write_all(b"hello").unwrap();
        z.start_file("bin/tool", opts).unwrap();
        z.write_all(b"\x00\x01").unwrap();
        z.start_file("../../evil.txt", opts).unwrap();
        z.write_all(b"evil").unwrap();
        z.finish().unwrap();
        p
    }

    #[tokio::test]
    async fn lists_and_extracts_zip_safely() {
        let dir = tempfile::tempdir().unwrap();
        let p = make_zip(dir.path());
        let l = list(&p, 100).await.unwrap();
        assert_eq!(l.format, "zip");
        assert_eq!(l.entries.len(), 4);
        assert!(l.supports_selective_extraction);
        assert_eq!(l.intact, Some(true));
        let out = dir.path().join("out");
        let n = extract(&p, Some(&["docs/".into()]), &out).await.unwrap();
        assert_eq!(n, 1);
        assert_eq!(
            std::fs::read(out.join("docs/readme.txt")).unwrap(),
            b"hello"
        );
        assert!(!out.join("bin/tool").exists());
        let n = extract(&p, None, &out).await.unwrap();
        // the traversal entry is skipped, never written anywhere
        assert_eq!(n, 2);
        assert!(!dir.path().join("evil.txt").exists());
        assert!(!out.join("evil.txt").exists());
        assert!(out.join("bin/tool").exists());
    }

    #[tokio::test]
    async fn tar_gz_round_trip_and_corrupt_detection() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.tar.gz");
        {
            let f = std::fs::File::create(&p).unwrap();
            let enc = flate2::write::GzEncoder::new(f, flate2::Compression::default());
            let mut b = tar::Builder::new(enc);
            let data = b"tar data";
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, "x/y.txt", &data[..]).unwrap();
            b.into_inner().unwrap().finish().unwrap();
        }
        let l = list(&p, 10).await.unwrap();
        assert_eq!(l.format, "tar.gz");
        assert_eq!(l.entries[0].path, "x/y.txt");
        let out = dir.path().join("o");
        assert_eq!(extract(&p, None, &out).await.unwrap(), 1);
        assert_eq!(std::fs::read(out.join("x/y.txt")).unwrap(), b"tar data");
        // corrupt: truncate
        let bytes = std::fs::read(&p).unwrap();
        let c = dir.path().join("c.tar.gz");
        std::fs::write(&c, &bytes[..bytes.len() / 2]).unwrap();
        let l = list(&c, 10).await.unwrap();
        assert_eq!(l.intact, Some(false));
        let r = dir.path().join("x.rar");
        std::fs::write(&r, b"Rar!\x1a\x07\x00").unwrap();
        let l = list(&r, 10).await.unwrap();
        assert_eq!(l.format, "rar");
        assert!(!l.supports_selective_extraction);
    }
}
