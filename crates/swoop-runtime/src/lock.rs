//! Single-instance lock so the desktop app and `swoop server` never open the same database
//! and torrent session concurrently.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

pub struct InstanceLock {
    _file: File,
}

impl InstanceLock {
    /// Try to take the lock. Returns `Ok(None)` if another process holds it.
    pub fn try_acquire(path: &Path) -> io::Result<Option<Self>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            // SAFETY: valid fd; flock is safe to call with these flags.
            let r = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if r != 0 {
                let e = io::Error::last_os_error();
                if e.raw_os_error() == Some(libc::EWOULDBLOCK) {
                    return Ok(None);
                }
                return Err(e);
            }
        }
        // Record the pid for diagnostics; the flock is what actually protects us.
        use std::io::Write;
        let mut f = &file;
        let _ = f.set_len(0);
        let _ = write!(f, "{}", std::process::id());
        Ok(Some(Self { _file: file }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_fails_while_held() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("swoop.lock");
        let a = InstanceLock::try_acquire(&p).unwrap();
        assert!(a.is_some());
        // flock is per open-file-description; a second open in the same process still conflicts.
        let b = InstanceLock::try_acquire(&p).unwrap();
        assert!(b.is_none());
        drop(a);
        // Another test may be mid `fork`/`exec`; the child briefly inherits our locked fd
        // (closed at exec by CLOEXEC), so allow the release a moment to become visible.
        let mut reacquired = false;
        for _ in 0..50 {
            if InstanceLock::try_acquire(&p).unwrap().is_some() {
                reacquired = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(reacquired, "lock not released after drop");
    }
}
