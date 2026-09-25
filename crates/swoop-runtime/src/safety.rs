//! Filesystem safety at trust boundaries: filename sanitisation, path-traversal rejection,
//! symlink escape checks. Every filename that comes from a server, a torrent, a playlist, an
//! archive listing or a remote client passes through here.

use std::path::{Component, Path, PathBuf};
use swoop_domain::{ErrorKind, TaskError};
use unicode_normalization::UnicodeNormalization;

/// Filesystem limit is 255 bytes; we reserve room for the part suffix (`.swoop-part`) and a
/// ` (9999)` uniqueness suffix so derived names never hit `ENAMETOOLONG`.
pub const MAX_FILENAME_BYTES: usize = 255 - 12 - 8;
pub const MAX_PATH_BYTES: usize = 1024;
pub const MAX_PATH_DEPTH: usize = 32;

const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Make an untrusted string safe to use as a single path component.
/// Never returns an empty string, `.` or `..`. Output is NFC-normalised so comparisons against
/// existing files behave the same on normalisation-insensitive filesystems (APFS) and others.
pub fn sanitize_filename(input: &str) -> String {
    let input: String = input.nfc().collect();
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        let replaced = match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}' => '_',
            c => c,
        };
        out.push(replaced);
    }
    // trim spaces and dots to a fixpoint (Windows strips them, macOS hides dotfiles)
    let mut trimmed = out.as_str();
    loop {
        let next = trimmed.trim().trim_end_matches('.').trim_start_matches('.');
        if next.len() == trimmed.len() {
            break;
        }
        trimmed = next;
    }
    let mut name = if trimmed.is_empty() {
        "download".to_owned()
    } else {
        trimmed.to_owned()
    };
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
    let mut depth = 0usize;
    for comp in Path::new(&raw).components() {
        match comp {
            Component::Normal(c) => {
                depth += 1;
                if depth > MAX_PATH_DEPTH {
                    return Err(TaskError::new(
                        ErrorKind::PathTraversal,
                        format!("path deeper than {MAX_PATH_DEPTH} components"),
                    ));
                }
                let s = c.to_string_lossy();
                out.push(sanitize_filename(&s));
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(TaskError::new(
                    ErrorKind::PathTraversal,
                    format!("rejected path component in {input:?}"),
                ));
            }
        }
    }
    if out.as_os_str().is_empty() {
        return Err(TaskError::new(ErrorKind::InvalidFilename, "empty path"));
    }
    if out.as_os_str().len() > MAX_PATH_BYTES {
        return Err(TaskError::new(ErrorKind::InvalidFilename, "path too long"));
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
        Err(TaskError::new(
            ErrorKind::PathTraversal,
            format!("{} escapes {}", candidate.display(), root.display()),
        ))
    }
}

/// Like [`ensure_within`] but `candidate` must be strictly below `root` (never `root` itself).
/// Use it before deleting anything: removing "the task's file" must never remove the whole
/// download folder because a path degenerated to the directory.
pub fn is_strictly_within(root: &Path, candidate: &Path) -> bool {
    let root_c = canonical_prefix(root);
    let cand_c = canonical_prefix(candidate);
    cand_c != root_c && cand_c.starts_with(&root_c)
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

/// Reject destination directories that are dangerous (system locations, launch agents, SSH
/// keys, shell config) or not absolute. Matching is prefix-based on the canonicalised path.
pub fn validate_destination_dir(dir: &Path) -> Result<(), TaskError> {
    if !dir.is_absolute() {
        return Err(TaskError::new(
            ErrorKind::InvalidFilename,
            "destination must be an absolute path",
        ));
    }
    if dir.as_os_str().len() > MAX_PATH_BYTES {
        return Err(TaskError::new(
            ErrorKind::InvalidFilename,
            "destination path too long",
        ));
    }
    let canon = canonical_prefix(dir);
    let s = canon.to_string_lossy().to_string();
    let normalised = s.trim_end_matches('/');
    let normalised = if normalised.is_empty() {
        "/"
    } else {
        normalised
    };
    let system_prefixes = [
        "/",
        "/System",
        "/bin",
        "/sbin",
        "/usr",
        "/etc",
        "/private/etc",
        "/Library",
        "/var",
        "/private/var",
        "/dev",
        "/cores",
        "/Applications",
        "/opt",
        "/boot",
        "/proc",
        "/sys",
        "/root",
        "C:\\Windows",
        "C:\\Program Files",
    ];
    for f in system_prefixes {
        if normalised.eq_ignore_ascii_case(f) {
            return Err(TaskError::new(
                ErrorKind::PermissionDenied,
                format!("refusing to download into {normalised}"),
            ));
        }
        // exact system dir or anything under it, except /Users, /home and /private/tmp trees
        if f != "/"
            && (normalised
                .to_ascii_lowercase()
                .starts_with(&format!("{}/", f.to_ascii_lowercase()))
                || normalised
                    .to_ascii_lowercase()
                    .starts_with(&format!("{}\\", f.to_ascii_lowercase())))
        {
            let lower = normalised.to_ascii_lowercase();
            let allowed = lower.starts_with("/private/tmp/")
                || lower.starts_with("/var/folders/")
                || lower.starts_with("/private/var/folders/")
                || lower.starts_with("/var/tmp/")
                || lower.starts_with("/private/var/tmp/")
                || lower.starts_with("/usr/local/")
                || lower.starts_with("/opt/")
                || lower.starts_with("/var/lib/swoop")
                || lower.starts_with("/var/swoop")
                || lower.starts_with("/private/var/lib/swoop");
            if !allowed {
                return Err(TaskError::new(
                    ErrorKind::PermissionDenied,
                    format!("refusing to download into {normalised}"),
                ));
            }
        }
    }
    // User-level sensitive locations (LaunchAgents can execute code at login; ~/.ssh holds keys).
    if let Some(home) = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()) {
        let home = canonical_prefix(&home).to_string_lossy().to_string();
        let home = home.trim_end_matches('/');
        let sensitive = [
            "/Library/LaunchAgents",
            "/Library/LaunchDaemons",
            "/.ssh",
            "/.gnupg",
            "/.config",
            "/.aws",
            "/.kube",
            "/Library/Application Support/com.apple.",
            "/Library/Preferences",
            "/Library/Keychains",
            "/.zshrc",
            "/.bashrc",
            "/.profile",
            "/.zprofile",
            "/Library/Application Scripts",
        ];
        for suffix in sensitive {
            let candidate = format!("{home}{suffix}");
            if normalised.eq_ignore_ascii_case(&candidate)
                || normalised
                    .to_ascii_lowercase()
                    .starts_with(&candidate.to_ascii_lowercase())
            {
                return Err(TaskError::new(
                    ErrorKind::PermissionDenied,
                    format!("refusing to download into {normalised}"),
                ));
            }
        }
        if normalised.eq_ignore_ascii_case(home) {
            // the home directory root itself is allowed (some users want ~), nothing to do
        }
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
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("download");
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
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
    format!(
        "{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    )
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
        assert_eq!(sanitize_filename(". .hidden"), "hidden");
        // NFD input is normalised to NFC
        assert_eq!(
            sanitize_filename("Re\u{0301}sume\u{0301}.pdf"),
            "Résumé.pdf"
        );
        let long = "x".repeat(300) + ".tar.gz";
        let t = sanitize_filename(&long);
        assert!(t.len() <= MAX_FILENAME_BYTES);
        assert!(t.ends_with(".gz"));
    }

    #[test]
    fn relative_paths() {
        assert_eq!(
            sanitize_relative_path("a/b/c.txt").unwrap(),
            PathBuf::from("a/b/c.txt")
        );
        assert!(sanitize_relative_path("../x").is_err());
        assert!(sanitize_relative_path(&"a/".repeat(40)).is_err());
        assert!(sanitize_relative_path("/abs").is_err());
        assert!(sanitize_relative_path("a\\..\\b").is_err());
        assert_eq!(
            sanitize_relative_path("./a/./b").unwrap(),
            PathBuf::from("a/b")
        );
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
        assert!(validate_destination_dir(Path::new("/Library/LaunchDaemons")).is_err());
        assert!(validate_destination_dir(Path::new("/usr/lib/x")).is_err());
        assert!(validate_destination_dir(Path::new("/usr/local/share")).is_ok());
        assert!(validate_destination_dir(Path::new("/private/tmp/swoop-test")).is_ok());
        if let Some(home) = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()) {
            assert!(validate_destination_dir(&home.join("Library/LaunchAgents")).is_err());
            assert!(validate_destination_dir(&home.join(".ssh")).is_err());
            assert!(validate_destination_dir(&home.join("Downloads")).is_ok());
        }
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
