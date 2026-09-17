//! Filesystem safety at trust boundaries: filename sanitisation, path-traversal rejection,
//! symlink escape checks. Every filename that comes from a server, a torrent, a playlist, an
//! archive listing or a remote client passes through here.

use osprey_domain::{ErrorKind, TaskError};
use std::path::{Component, Path, PathBuf};

pub const MAX_FILENAME_BYTES: usize = 255;

const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4",
    "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Make an untrusted string safe to use as a single path component.
/// Never returns an empty string, `.` or `..`.
pub fn sanitize_filename(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        let replaced = match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}' => '_',
            c => c,
        };
        out.push(replaced);
    }
    // trim spaces and dots (Windows strips them, macOS hides dotfiles)
    let trimmed = out.trim().trim_end_matches('.').trim_start_matches('.').trim().to_owned();
    let mut name = if trimmed.is_empty() { "download".to_owned() } else { trimmed };
    let stem_upper = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if WINDOWS_RESERVED.contains(&stem_upper.as_str()) {
        name = format!("_{name}");
    }
    if name == "." || name == ".." {
        name = "download".to_owned();
    }
    truncate_filename(&name, MAX_FILENAME_BYTES)
}

/// Truncate keeping the extension and respecting UTF-8 boundaries.
pub fn truncate_filename(name: &str, max_bytes: usize) -> String {
    if name.len() <= max_bytes {
        return name.to_owned();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 && name.len() - i <= 16 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let budget = max_bytes.saturating_sub(ext.len()).max(1);
    let mut cut = budget.min(stem.len());
    while cut > 0 && !stem.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{}", &stem[..cut], ext)
}

/// Validate a relative path (e.g. from a torrent or archive) and return a normalised version
/// that cannot escape its root.
pub fn sanitize_relative_path(input: &str) -> Result<PathBuf, TaskError> {
    let raw = input.replace('\\', "/");
    let mut out = PathBuf::new();
    for comp in Path::new(&raw).components() {
        match comp {
            Component::Normal(c) => {
                let s = c.to_string_lossy();
                out.push(sanitize_filename(&s));
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(TaskError::new(ErrorKind::PathTraversal, format!("rejected path component in {input:?}")));
            }
        }
    }
    if out.as_os_str().is_empty() {
        return Err(TaskError::new(ErrorKind::InvalidFilename, "empty path"));
    }
    Ok(out)
}

/// Ensure `candidate` is inside `root` after resolving symlinks of the existing prefix.
/// Returns the canonical root-joined path.
pub fn ensure_within(root: &Path, candidate: &Path) -> Result<PathBuf, TaskError> {
    let root_c = canonical_prefix(root);
    let cand_c = canonical_prefix(candidate);
    if cand_c.starts_with(&root_c) {
        Ok(cand_c)
    } else {
        Err(TaskError::new(ErrorKind::PathTraversal, format!("{} escapes {}", candidate.display(), root.display())))
    }
}

/// Canonicalise the longest existing prefix of `p` and append the rest lexically normalised.
pub fn canonical_prefix(p: &Path) -> PathBuf {
    let mut existing = p.to_path_buf();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() {
        match (existing.file_name(), existing.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name.to_owned());
                existing = parent.to_path_buf();
            }
            _ => break,
        }
    }
    let mut base = existing.canonicalize().unwrap_or(existing);
    for r in rest.iter().rev() {
        base.push(r);
    }
    base
}

/// Reject destination directories that are dangerous (root, system dirs) or not absolute.
pub fn validate_destination_dir(dir: &Path) -> Result<(), TaskError> {
    if !dir.is_absolute() {
        return Err(TaskError::new(ErrorKind::InvalidFilename, "destination must be an absolute path"));
    }
    let s = dir.to_string_lossy();
    let forbidden = ["/", "/System", "/bin", "/sbin", "/usr", "/etc", "/private/etc", "/Library", "/var", "/private/var/root", "C:\\Windows", "C:\\"];
    let normalised = s.trim_end_matches('/');
    let normalised = if normalised.is_empty() { "/" } else { normalised };
    if forbidden.iter().any(|f| normalised.eq_ignore_ascii_case(f)) {
        return Err(TaskError::new(ErrorKind::PermissionDenied, format!("refusing to download into {normalised}")));
    }
    Ok(())
}

/// Is `path` a symlink pointing outside `root`? Used before writing into an existing path.
pub fn symlink_escapes(root: &Path, path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => match std::fs::canonicalize(path) {
            Ok(target) => !target.starts_with(canonical_prefix(root)),
            Err(_) => true,
        },
        _ => false,
    }
}

/// Generate ` (2)`, ` (3)` … variants until one does not exist.
pub fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("download");
    let ext = path.extension().and_then(|s| s.to_str()).map(|e| format!(".{e}")).unwrap_or_default();
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    for n in 2..10_000 {
        let candidate = parent.join(format!("{stem} ({n}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{stem} ({}){ext}", uuid_suffix()))
}

fn uuid_suffix() -> String {
    format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitises_names() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(sanitize_filename("  "), "download");
        assert_eq!(sanitize_filename("con.txt"), "_con.txt");
        assert_eq!(sanitize_filename("a\u{202E}b.exe"), "a_b.exe");
        assert_eq!(sanitize_filename("..."), "download");
        assert_eq!(sanitize_filename(".hidden"), "hidden");
        let long = "x".repeat(300) + ".tar.gz";
        let t = sanitize_filename(&long);
        assert!(t.len() <= 255);
        assert!(t.ends_with(".gz"));
    }

    #[test]
    fn relative_paths() {
        assert_eq!(sanitize_relative_path("a/b/c.txt").unwrap(), PathBuf::from("a/b/c.txt"));
        assert!(sanitize_relative_path("../x").is_err());
        assert!(sanitize_relative_path("/abs").is_err());
        assert_eq!(sanitize_relative_path("a\\..\\b").is_err(), true);
        assert_eq!(sanitize_relative_path("./a/./b").unwrap(), PathBuf::from("a/b"));
    }

    #[test]
    fn within_root() {
        let dir = tempfile::tempdir().unwrap();
        let ok = ensure_within(dir.path(), &dir.path().join("sub/file"));
        assert!(ok.is_ok());
        let bad = ensure_within(dir.path(), &dir.path().join("../escape"));
        assert!(bad.is_err());
    }

    #[test]
    fn destination_validation() {
        assert!(validate_destination_dir(Path::new("/")).is_err());
        assert!(validate_destination_dir(Path::new("/System")).is_err());
        assert!(validate_destination_dir(Path::new("relative")).is_err());
        assert!(validate_destination_dir(Path::new("/Users/me/Downloads")).is_ok());
    }

    #[test]
    fn unique_names() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        std::fs::write(&p, b"x").unwrap();
        let u = unique_path(&p);
        assert_eq!(u.file_name().unwrap().to_str().unwrap(), "a (2).txt");
    }
}
