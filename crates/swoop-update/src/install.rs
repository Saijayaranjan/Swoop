//! Installing a verified update on macOS, in two halves:
//!
//! 1. [`stage_update`] runs inside the app. It re-checks the DMG's Ed25519 signature, mounts the
//!    image read-only, requires exactly one `Swoop.app` with the right bundle identifier, a newer
//!    version and a valid code signature, and copies it with `ditto` into a staging folder next
//!    to the running bundle (same volume, so the later swap is two atomic renames).
//! 2. [`apply_update`] runs in a small detached helper (`swoop update apply`, hidden) after the
//!    app has quit cleanly. It waits for the app's PID to exit, moves the old bundle aside, moves
//!    the new one into place, strips quarantine, relaunches, and rolls back if any step fails.
//!
//! Nothing here runs on an unverified image: the signature check happens before `hdiutil`.

use crate::signing;
use ed25519_dalek::VerifyingKey;
use semver::Version;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The only bundle identifier an update may carry.
pub const APP_BUNDLE_ID: &str = "app.swoop.desktop";
/// The only app name an update image may contain.
pub const APP_NAME: &str = "Swoop.app";
/// Hidden folder created next to the running bundle for the staged copy and the backup.
pub const STAGING_DIR: &str = ".Swoop-update";

// ---------------------------------------------------------------------------------------------
// Stage (in the app)
// ---------------------------------------------------------------------------------------------

pub struct StageRequest<'a> {
    /// The downloaded, already-verified DMG. Its signature is checked again before mounting.
    pub dmg: &'a Path,
    /// Contents of the release's `.sig` asset.
    pub signature_b64: &'a str,
    pub key: &'a VerifyingKey,
    /// The running app's version; the update must be strictly newer.
    pub current_version: &'a Version,
    /// The release's version; the app inside the image must match it.
    pub expected_version: Option<&'a Version>,
    /// The running `Swoop.app`.
    pub current_bundle: &'a Path,
}

/// Verify, mount, validate and stage an update. Returns the staged `Swoop.app`.
pub fn stage_update(req: &StageRequest<'_>) -> Result<PathBuf, String> {
    let parent = check_location(req.current_bundle)?;
    signing::verify_file(req.key, req.dmg, req.signature_b64).map_err(|e| e.message)?;

    let mount = Mount::attach(req.dmg)?;
    let app = find_single_app(&mount.point)?;
    validate_app(&app, req.current_version, req.expected_version)?;

    let staging = parent.join(STAGING_DIR);
    if staging.exists() {
        std::fs::remove_dir_all(&staging)
            .map_err(|e| format!("couldn't clear the old staging folder: {e}"))?;
    }
    std::fs::create_dir(&staging)
        .map_err(|e| format!("couldn't create {}: {e}", staging.display()))?;
    let staged = staging.join(APP_NAME);
    let result = (|| {
        run(Command::new("/usr/bin/ditto")
            .arg("--noqtn")
            .arg(&app)
            .arg(&staged))
        .map_err(|e| format!("couldn't copy the update: {e}"))?;
        // The copy must be exactly as trustworthy as the original.
        validate_app(&staged, req.current_version, req.expected_version)?;
        strip_quarantine(&staged);
        Ok(staged.clone())
    })();
    drop(mount);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
}

/// The bundle's folder must be writable (and not an App Translocation or read-only mount), or
/// the swap can't happen: say so plainly.
pub fn check_location(bundle: &Path) -> Result<PathBuf, String> {
    let bundle = bundle
        .canonicalize()
        .map_err(|e| format!("can't locate the running app: {e}"))?;
    let parent = bundle
        .parent()
        .ok_or("the running app has no parent folder")?
        .to_path_buf();
    let translocated = bundle.to_string_lossy().contains("/AppTranslocation/");
    if translocated || !writable(&parent) || !writable(&bundle) {
        return Err(format!(
            "Swoop can't update itself in {} because that location isn't writable. Move Swoop to \
             your Applications folder and try again.",
            parent.display()
        ));
    }
    Ok(parent)
}

fn writable(p: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(p.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `c` is a valid NUL-terminated path for the duration of the call.
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

/// A read-only, hidden mount of the DMG, detached on drop.
struct Mount {
    point: PathBuf,
}

impl Mount {
    fn attach(dmg: &Path) -> Result<Self, String> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let point =
            std::env::temp_dir().join(format!("swoop-update-mount-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&point).map_err(|e| format!("mount point: {e}"))?;
        let r = run(Command::new("/usr/bin/hdiutil")
            .args([
                "attach",
                "-nobrowse",
                "-readonly",
                "-noautoopen",
                "-mountpoint",
            ])
            .arg(&point)
            .arg(dmg));
        if let Err(e) = r {
            let _ = std::fs::remove_dir(&point);
            return Err(format!("couldn't open the update disk image: {e}"));
        }
        Ok(Self { point })
    }
}

impl Drop for Mount {
    fn drop(&mut self) {
        let ok = run(Command::new("/usr/bin/hdiutil")
            .args(["detach", "-quiet"])
            .arg(&self.point))
        .is_ok();
        if !ok {
            let _ = run(Command::new("/usr/bin/hdiutil")
                .args(["detach", "-force", "-quiet"])
                .arg(&self.point));
        }
        let _ = std::fs::remove_dir(&self.point);
    }
}

/// Exactly one `.app` at the top of the image, and it must be `Swoop.app`.
fn find_single_app(root: &Path) -> Result<PathBuf, String> {
    let entries = std::fs::read_dir(root).map_err(|e| format!("read disk image: {e}"))?;
    let apps: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        // `file_type` doesn't follow symlinks, so the Applications alias never counts.
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("app")))
        .collect();
    match apps.as_slice() {
        [one] if one.file_name().is_some_and(|n| n == APP_NAME) => Ok(one.clone()),
        [other] => Err(format!(
            "the update contains {} instead of {APP_NAME}",
            other.file_name().unwrap_or_default().to_string_lossy()
        )),
        _ => Err(format!(
            "the update must contain exactly one app; found {}",
            apps.len()
        )),
    }
}

/// Read one key from an app's Info.plist.
pub fn plist_value(app: &Path, key: &str) -> Option<String> {
    let out = Command::new("/usr/bin/plutil")
        .args(["-extract", key, "raw", "-o", "-"])
        .arg(app.join("Contents/Info.plist"))
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// Bundle identifier, version (strictly newer, and the release's if known) and code signature.
pub fn validate_app(
    app: &Path,
    current: &Version,
    expected: Option<&Version>,
) -> Result<Version, String> {
    let id = plist_value(app, "CFBundleIdentifier").unwrap_or_default();
    if id != APP_BUNDLE_ID {
        return Err(format!(
            "the update's bundle identifier is {id:?}, not {APP_BUNDLE_ID}"
        ));
    }
    let raw = plist_value(app, "CFBundleShortVersionString").unwrap_or_default();
    let version = crate::version::parse_version(&raw)
        .ok_or_else(|| format!("the update has an unreadable version {raw:?}"))?;
    if !crate::version::is_newer(&version, current) {
        return Err(format!(
            "the update's version {version} isn't newer than {current}"
        ));
    }
    if let Some(exp) = expected {
        if version != *exp {
            return Err(format!(
                "the update's app is version {version} but the release is {exp}"
            ));
        }
    }
    run(Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(app))
    .map_err(|e| format!("the update's code signature is invalid: {e}"))?;
    Ok(version)
}

/// Remove `com.apple.quarantine` recursively (absent attributes are fine).
pub fn strip_quarantine(path: &Path) {
    let _ = Command::new("/usr/bin/xattr")
        .args(["-dr", "com.apple.quarantine"])
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Run a command; Err carries its stderr (or exit status).
fn run(cmd: &mut Command) -> Result<(), String> {
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        Err(if err.is_empty() {
            out.status.to_string()
        } else {
            err
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Apply (in the detached helper)
// ---------------------------------------------------------------------------------------------

pub struct ApplyRequest {
    /// The app process to wait for; `None` = don't wait.
    pub pid: Option<i32>,
    /// The staged bundle (`<parent>/.Swoop-update/Swoop.app`).
    pub staged: PathBuf,
    /// Where the app lives (`/Applications/Swoop.app`).
    pub target: PathBuf,
    pub relaunch: bool,
    pub wait_timeout: Duration,
    pub launch_timeout: Duration,
}

impl ApplyRequest {
    pub fn new(pid: Option<i32>, staged: PathBuf, target: PathBuf, relaunch: bool) -> Self {
        Self {
            pid,
            staged,
            target,
            relaunch,
            wait_timeout: Duration::from_secs(180),
            launch_timeout: Duration::from_secs(30),
        }
    }
}

/// Detach from the launching app's session so quitting it can't take the helper down.
pub fn detach_session() {
    // SAFETY: setsid has no memory-safety preconditions; failure (already a leader) is harmless.
    unsafe {
        libc::setsid();
    }
}

fn process_alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks for existence/permission.
    let r = unsafe { libc::kill(pid, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Swap the staged bundle into place and relaunch, rolling back on failure. `log` receives
/// one line per step.
pub fn apply_update(req: &ApplyRequest, log: &mut dyn FnMut(&str)) -> Result<(), String> {
    let staging = req
        .staged
        .parent()
        .ok_or("staged bundle has no parent")?
        .to_path_buf();
    let fail = |msg: String, log: &mut dyn FnMut(&str)| {
        log(&format!("error: {msg}"));
        Err(msg)
    };

    if let Some(pid) = req.pid {
        log(&format!("waiting for Swoop (pid {pid}) to quit"));
        let start = Instant::now();
        while process_alive(pid) {
            if start.elapsed() > req.wait_timeout {
                let _ = std::fs::remove_dir_all(&staging);
                return fail(
                    format!("Swoop (pid {pid}) didn't quit; update abandoned"),
                    log,
                );
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    if plist_value(&req.staged, "CFBundleIdentifier").as_deref() != Some(APP_BUNDLE_ID) {
        let _ = std::fs::remove_dir_all(&staging);
        return fail(
            format!("{} isn't a staged Swoop update", req.staged.display()),
            log,
        );
    }
    if req.target.file_name().is_none_or(|n| {
        !Path::new(n)
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("app"))
    }) {
        return fail(
            format!("{} isn't an app bundle path", req.target.display()),
            log,
        );
    }

    let backup = staging.join("Swoop.previous.app");
    let had_old = req.target.exists();
    if had_old {
        let _ = std::fs::remove_dir_all(&backup);
        if let Err(e) = std::fs::rename(&req.target, &backup) {
            let _ = std::fs::remove_dir_all(&staging);
            return fail(format!("couldn't move the old Swoop aside: {e}"), log);
        }
        log(&format!("moved old bundle to {}", backup.display()));
    }
    if let Err(e) = std::fs::rename(&req.staged, &req.target) {
        if had_old {
            let _ = std::fs::rename(&backup, &req.target);
        }
        return fail(format!("couldn't move the new Swoop into place: {e}"), log);
    }
    log(&format!("installed {}", req.target.display()));
    strip_quarantine(&req.target);

    if req.relaunch {
        if let Err(e) = launch_and_confirm(&req.target, req.launch_timeout) {
            log(&format!("new version failed to launch ({e}); rolling back"));
            if had_old {
                let failed = staging.join("Swoop.failed.app");
                let _ = std::fs::remove_dir_all(&failed);
                let moved = std::fs::rename(&req.target, &failed)
                    .and_then(|_| std::fs::rename(&backup, &req.target));
                match moved {
                    Ok(()) => {
                        log("restored the previous version");
                        let _ = launch_and_confirm(&req.target, req.launch_timeout);
                        let _ = std::fs::remove_dir_all(&staging);
                    }
                    Err(e2) => log(&format!(
                        "rollback failed ({e2}); the previous version is at {}",
                        backup.display()
                    )),
                }
            }
            return fail(format!("the new version didn't launch: {e}"), log);
        }
        log("relaunched");
    }
    if let Err(e) = std::fs::remove_dir_all(&staging) {
        log(&format!("couldn't remove {}: {e}", staging.display()));
    }
    log("done");
    Ok(())
}

/// `open` the bundle and wait until its executable is running.
fn launch_and_confirm(app: &Path, timeout: Duration) -> Result<(), String> {
    run(Command::new("/usr/bin/open").arg(app))?;
    let exe_name = plist_value(app, "CFBundleExecutable").unwrap_or_else(|| "Swoop".into());
    let exe = app.join("Contents/MacOS").join(exe_name);
    let pattern = regex_escape(&exe.to_string_lossy());
    let start = Instant::now();
    while start.elapsed() < timeout {
        let found = Command::new("/usr/bin/pgrep")
            .arg("-f")
            .arg(&pattern)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if found {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(format!("{} didn't start within {timeout:?}", exe.display()))
}

fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if "\\.+*?()|[]{}^$".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
