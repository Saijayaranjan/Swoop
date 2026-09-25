//! Positional file writer used by the HTTP/FTP/HLS engines.
//!
//! One dedicated blocking thread per open file receives `(offset, bytes)` messages over a
//! channel bounded **by bytes** (natural back-pressure, bounded memory), merges adjacent writes,
//! performs `pwrite`, and answers `flush` requests so callers can advance their *committed*
//! watermark safely. Errors are sticky: after the first failure every operation returns it
//! and the writer must be reopened.
//!
//! Flush levels: [`FlushLevel::Barrier`] (`F_BARRIERFSYNC` on macOS, `fdatasync` elsewhere —
//! cheap, ordered; used for periodic checkpoints and pause) and [`FlushLevel::Full`]
//! (`F_FULLFSYNC` on macOS — used once before the final rename).

use bytes::Bytes;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use swoop_domain::{ErrorKind, TaskError};
use tokio::sync::{mpsc, oneshot, Semaphore};

#[cfg(unix)]
use std::os::unix::fs::FileExt;
#[cfg(windows)]
use std::os::windows::fs::FileExt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlushLevel {
    /// Orders writes and pushes them to the device queue; survives an app crash and, on most
    /// hardware, a power loss shortly after.
    Barrier,
    /// Forces the drive cache to stable media. Expensive; use before renaming to the final name.
    Full,
}

enum Cmd {
    Write {
        offset: u64,
        data: Bytes,
        permits: u32,
    },
    Flush {
        level: FlushLevel,
        ack: oneshot::Sender<io::Result<()>>,
    },
    Truncate {
        len: u64,
        ack: oneshot::Sender<io::Result<()>>,
    },
    Close {
        ack: oneshot::Sender<io::Result<()>>,
    },
}

/// Maximum bytes queued to the writer thread per file.
pub const DEFAULT_IN_FLIGHT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone)]
pub struct FileWriter {
    tx: mpsc::UnboundedSender<Cmd>,
    inflight: Arc<Semaphore>,
    path: Arc<PathBuf>,
}

#[derive(Default)]
pub struct OpenOptionsExt {
    pub preallocate: Option<u64>,
    pub sparse: bool,
    /// Bytes allowed in flight before `write` awaits (default 4 MiB).
    pub in_flight_bytes: Option<usize>,
    /// If set, the file's parent directory must resolve (symlinks followed) to a location
    /// inside this root. The user's chosen destination may itself be a symlink (a Downloads
    /// folder pointing at an external drive is common) — that is fine; what we refuse is a
    /// symlink *inside* the managed tree pointing elsewhere.
    pub expected_root: Option<PathBuf>,
}

impl FileWriter {
    /// Open (or create) `path` for positional writes. The leaf is opened with `O_NOFOLLOW`
    /// (a symlink in place of the target file is refused) and, when `expected_root` is given,
    /// the resolved parent directory must lie inside it.
    pub async fn open(path: &Path, opts: OpenOptionsExt) -> Result<Self, TaskError> {
        let path_buf = path.to_path_buf();
        let expected_root = opts.expected_root.clone();
        let file = tokio::task::spawn_blocking({
            let p = path_buf.clone();
            move || -> io::Result<File> {
                if let Some(parent) = p.parent() {
                    std::fs::create_dir_all(parent)?;
                    if let Some(root) = &expected_root {
                        let root_c = std::fs::canonicalize(root).unwrap_or_else(|_| root.clone());
                        let parent_c = std::fs::canonicalize(parent)?;
                        if !parent_c.starts_with(&root_c) {
                            return Err(io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                format!("{} escapes {}", parent.display(), root.display()),
                            ));
                        }
                    }
                }
                let f = open_nofollow(&p)?;
                if let Some(len) = opts.preallocate {
                    preallocate(&f, len, opts.sparse)?;
                }
                Ok(f)
            }
        })
        .await
        .map_err(|e| TaskError::internal(format!("open join: {e}")))?
        .map_err(|e| TaskError::from_io(&e, &format!("open {}", path.display())))?;

        let (tx, mut rx) = mpsc::unbounded_channel::<Cmd>();
        let cap = opts
            .in_flight_bytes
            .unwrap_or(DEFAULT_IN_FLIGHT_BYTES)
            .max(256 * 1024);
        let inflight = Arc::new(Semaphore::new(cap));
        let inflight_thread = inflight.clone();
        let p = path_buf.clone();
        std::thread::Builder::new()
            .name(format!(
                "swoop-writer-{}",
                p.file_name().and_then(|s| s.to_str()).unwrap_or("file")
            ))
            .spawn(move || {
                let mut sticky: Option<io::Error> = None;
                // pending contiguous run to coalesce small chunks into one pwrite
                let mut run_offset: u64 = 0;
                let mut run: Vec<u8> = Vec::with_capacity(512 * 1024);
                let mut run_permits: u32 = 0;
                let flush_run = |run: &mut Vec<u8>,
                                 run_offset: u64,
                                 run_permits: &mut u32,
                                 sticky: &mut Option<io::Error>| {
                    if !run.is_empty() {
                        if sticky.is_none() {
                            if let Err(e) = write_all_at(&file, run, run_offset) {
                                *sticky = Some(e);
                            }
                        }
                        run.clear();
                    }
                    if *run_permits > 0 {
                        inflight_thread.add_permits(*run_permits as usize);
                        *run_permits = 0;
                    }
                };
                while let Some(cmd) = rx.blocking_recv() {
                    match cmd {
                        Cmd::Write {
                            offset,
                            data,
                            permits,
                        } => {
                            let contiguous =
                                !run.is_empty() && run_offset + run.len() as u64 == offset;
                            if !contiguous || run.len() + data.len() > 1024 * 1024 {
                                flush_run(&mut run, run_offset, &mut run_permits, &mut sticky);
                                run_offset = offset;
                            }
                            run.extend_from_slice(&data);
                            run_permits += permits;
                            if run.len() >= 512 * 1024 {
                                flush_run(&mut run, run_offset, &mut run_permits, &mut sticky);
                            }
                        }
                        Cmd::Flush { level, ack } => {
                            flush_run(&mut run, run_offset, &mut run_permits, &mut sticky);
                            let r = match &sticky {
                                Some(e) => Err(clone_err(e)),
                                None => sync(&file, level),
                            };
                            if let Err(e) = &r {
                                if sticky.is_none() {
                                    sticky = Some(clone_err(e));
                                }
                            }
                            let _ = ack.send(r);
                        }
                        Cmd::Truncate { len, ack } => {
                            flush_run(&mut run, run_offset, &mut run_permits, &mut sticky);
                            let r = match &sticky {
                                Some(e) => Err(clone_err(e)),
                                None => file.set_len(len),
                            };
                            let _ = ack.send(r);
                        }
                        Cmd::Close { ack } => {
                            flush_run(&mut run, run_offset, &mut run_permits, &mut sticky);
                            let r = match &sticky {
                                Some(e) => Err(clone_err(e)),
                                None => sync(&file, FlushLevel::Barrier),
                            };
                            let _ = ack.send(r);
                            break;
                        }
                    }
                }
            })
            .map_err(|e| TaskError::internal(format!("spawn writer: {e}")))?;

        Ok(Self {
            tx,
            inflight,
            path: Arc::new(path_buf),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Queue a write. Awaits while more than the in-flight budget is queued (back-pressure).
    pub async fn write(&self, offset: u64, data: Bytes) -> Result<(), TaskError> {
        let n = data.len().min(u32::MAX as usize) as u32;
        // A single write larger than the budget is allowed through in one piece.
        let want = (n as usize).min(self.inflight_capacity()) as u32;
        let permit = self
            .inflight
            .acquire_many(want)
            .await
            .map_err(|_| TaskError::new(ErrorKind::DiskWriteError, "writer closed"))?;
        permit.forget();
        self.tx
            .send(Cmd::Write {
                offset,
                data,
                permits: want,
            })
            .map_err(|_| TaskError::new(ErrorKind::DiskWriteError, "writer thread stopped"))
    }

    fn inflight_capacity(&self) -> usize {
        DEFAULT_IN_FLIGHT_BYTES.max(256 * 1024)
    }

    /// Flush everything queued so far. Returns the sticky error if the writer failed earlier.
    pub async fn flush(&self, level: FlushLevel) -> Result<(), TaskError> {
        let (ack, rx) = oneshot::channel();
        self.tx
            .send(Cmd::Flush { level, ack })
            .map_err(|_| TaskError::new(ErrorKind::DiskWriteError, "writer thread stopped"))?;
        rx.await
            .map_err(|_| TaskError::new(ErrorKind::DiskWriteError, "writer thread stopped"))?
            .map_err(|e| TaskError::from_io(&e, "flush"))
    }

    pub async fn truncate(&self, len: u64) -> Result<(), TaskError> {
        let (ack, rx) = oneshot::channel();
        self.tx
            .send(Cmd::Truncate { len, ack })
            .map_err(|_| TaskError::new(ErrorKind::DiskWriteError, "writer thread stopped"))?;
        rx.await
            .map_err(|_| TaskError::new(ErrorKind::DiskWriteError, "writer thread stopped"))?
            .map_err(|e| TaskError::from_io(&e, "truncate"))
    }

    /// Flush (barrier) and close. The writer is unusable afterwards.
    pub async fn close(self) -> Result<(), TaskError> {
        let (ack, rx) = oneshot::channel();
        if self.tx.send(Cmd::Close { ack }).is_err() {
            return Ok(());
        }
        rx.await
            .map_err(|_| TaskError::new(ErrorKind::DiskWriteError, "writer thread stopped"))?
            .map_err(|e| TaskError::from_io(&e, "close"))
    }
}

fn clone_err(e: &io::Error) -> io::Error {
    match e.raw_os_error() {
        Some(code) => io::Error::from_raw_os_error(code),
        None => io::Error::new(e.kind(), e.to_string()),
    }
}

fn write_all_at(file: &File, mut data: &[u8], mut offset: u64) -> io::Result<()> {
    while !data.is_empty() {
        #[cfg(unix)]
        let n = file.write_at(data, offset)?;
        #[cfg(windows)]
        let n = file.seek_write(data, offset)?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "write returned 0"));
        }
        data = &data[n..];
        offset += n as u64;
    }
    Ok(())
}

fn sync(file: &File, level: FlushLevel) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::io::AsRawFd;
        let cmd = match level {
            FlushLevel::Barrier => libc::F_BARRIERFSYNC,
            FlushLevel::Full => libc::F_FULLFSYNC,
        };
        // SAFETY: valid fd; fcntl with these commands takes no pointer argument.
        let r = unsafe { libc::fcntl(file.as_raw_fd(), cmd) };
        if r == -1 {
            let e = io::Error::last_os_error();
            // Network/FAT volumes may not support the barrier; fall back to fsync.
            if e.raw_os_error() == Some(libc::ENOTSUP) || e.raw_os_error() == Some(libc::EINVAL) {
                return file.sync_data();
            }
            return Err(e);
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        match level {
            FlushLevel::Barrier => file.sync_data(),
            FlushLevel::Full => file.sync_all(),
        }
    }
}

fn open_nofollow(p: &Path) -> io::Result<File> {
    let mut o = OpenOptions::new();
    o.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        o.custom_flags(libc::O_NOFOLLOW);
    }
    o.open(p)
}

/// Reserve space up front. `sparse` only sets the length (cheap, instant); otherwise we ask the
/// filesystem to actually allocate blocks so `ENOSPC` surfaces before the transfer starts.
pub fn preallocate(file: &File, len: u64, sparse: bool) -> io::Result<()> {
    if len == 0 {
        return Ok(());
    }
    if sparse {
        return file.set_len(len);
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::io::AsRawFd;
        let mut store = libc::fstore_t {
            fst_flags: libc::F_ALLOCATECONTIG,
            fst_posmode: libc::F_PEOFPOSMODE,
            fst_offset: 0,
            fst_length: len as libc::off_t,
            fst_bytesalloc: 0,
        };
        // SAFETY: fstore_t is fully initialised and the fd is valid for the lifetime of `file`.
        let mut r = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_PREALLOCATE, &mut store) };
        if r == -1 {
            store.fst_flags = libc::F_ALLOCATEALL;
            r = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_PREALLOCATE, &mut store) };
        }
        if r == -1 {
            let e = io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::ENOSPC) {
                return Err(e);
            }
            // Filesystem does not support F_PREALLOCATE (network volumes): fall back to set_len.
        }
        return file.set_len(len);
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::io::AsRawFd;
        // SAFETY: valid fd, plain integer arguments.
        let r = unsafe { libc::fallocate(file.as_raw_fd(), 0, 0, len as libc::off_t) };
        if r == -1 {
            let e = io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::ENOSPC) {
                return Err(e);
            }
        }
        return file.set_len(len);
    }
    #[allow(unreachable_code)]
    file.set_len(len)
}

/// Free space on the volume containing `path` (walks up to the nearest existing ancestor).
pub fn free_space(path: &Path) -> Option<u64> {
    volume_stats(path).map(|(free, _)| free)
}

/// (free, total) bytes on the volume containing `path`.
pub fn volume_stats(path: &Path) -> Option<(u64, u64)> {
    let mut p = path.to_path_buf();
    while !p.exists() {
        p = p.parent()?.to_path_buf();
    }
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let c = CString::new(p.as_os_str().as_bytes()).ok()?;
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: c is a valid NUL-terminated path and st is a valid out-pointer.
        let r = unsafe { libc::statvfs(c.as_ptr(), &mut st) };
        if r != 0 {
            return None;
        }
        Some((
            st.f_bavail as u64 * st.f_frsize as u64,
            st.f_blocks as u64 * st.f_frsize as u64,
        ))
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// Apply the macOS quarantine attribute so Gatekeeper treats the file as downloaded.
/// No-op on other platforms.
pub fn set_quarantine(path: &Path, agent: &str, origin_url: Option<&str>) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        // Format: flags;timestamp-hex;agent;uuid — the same layout the system writes.
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let value = format!("0083;{ts:08x};{agent};");
        let _ = origin_url; // origin is recorded by LaunchServices when the app has LSFileQuarantineEnabled
        let c = CString::new(path.as_os_str().as_bytes())?;
        let name = CString::new("com.apple.quarantine")?;
        // SAFETY: all pointers are valid NUL-terminated strings / buffers for the call.
        let r = unsafe {
            libc::setxattr(
                c.as_ptr(),
                name.as_ptr(),
                value.as_ptr() as *const _,
                value.len(),
                0,
                0,
            )
        };
        if r != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, agent, origin_url);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writes_out_of_order_and_flushes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("out.bin");
        let w = FileWriter::open(
            &p,
            OpenOptionsExt {
                preallocate: Some(10),
                sparse: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        w.write(5, Bytes::from_static(b"56789")).await.unwrap();
        w.write(0, Bytes::from_static(b"01234")).await.unwrap();
        w.flush(FlushLevel::Barrier).await.unwrap();
        w.flush(FlushLevel::Full).await.unwrap();
        w.close().await.unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"0123456789");
        assert!(free_space(dir.path()).unwrap() > 0);
    }

    #[tokio::test]
    async fn coalesces_contiguous_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.bin");
        let w = FileWriter::open(&p, OpenOptionsExt::default())
            .await
            .unwrap();
        let mut expected = Vec::new();
        for i in 0..2000u32 {
            let chunk = vec![(i % 251) as u8; 1000];
            expected.extend_from_slice(&chunk);
            w.write(i as u64 * 1000, Bytes::from(chunk)).await.unwrap();
        }
        w.close().await.unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), expected);
    }

    #[tokio::test]
    async fn preallocates_real_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("pre.bin");
        let w = FileWriter::open(
            &p,
            OpenOptionsExt {
                preallocate: Some(1 << 20),
                sparse: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        w.close().await.unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().len(), 1 << 20);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn refuses_symlink_escapes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        // a symlinked *directory* inside the managed tree pointing outside it
        std::os::unix::fs::symlink(&outside, root.join("sub")).unwrap();
        let r = FileWriter::open(
            &root.join("sub/f.bin"),
            OpenOptionsExt {
                expected_root: Some(root.clone()),
                ..Default::default()
            },
        )
        .await;
        assert!(
            r.is_err(),
            "directory symlink escaping the root must be refused"
        );
        // a symlinked *file* in place of the target
        std::fs::write(outside.join("victim"), b"v").unwrap();
        std::os::unix::fs::symlink(outside.join("victim"), root.join("g.bin")).unwrap();
        let r = FileWriter::open(&root.join("g.bin"), OpenOptionsExt::default()).await;
        assert!(r.is_err(), "O_NOFOLLOW must refuse a symlinked leaf");
        // the root itself being a symlink is allowed
        let link_root = dir.path().join("link-root");
        std::os::unix::fs::symlink(&root, &link_root).unwrap();
        let w = FileWriter::open(
            &link_root.join("h.bin"),
            OpenOptionsExt {
                expected_root: Some(link_root.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        w.close().await.unwrap();
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn quarantine_attribute() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("q.bin");
        std::fs::write(&p, b"x").unwrap();
        set_quarantine(&p, "Swoop", None).unwrap();
        let out = std::process::Command::new("xattr")
            .arg("-p")
            .arg("com.apple.quarantine")
            .arg(&p)
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains("Swoop"));
    }
}
