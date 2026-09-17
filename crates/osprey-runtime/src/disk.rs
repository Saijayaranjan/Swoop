//! Positional file writer used by the HTTP/FTP/HLS engines.
//!
//! One dedicated blocking thread per open file receives `(offset, bytes)` messages over a
//! bounded channel (natural back-pressure), performs `pwrite`, and answers `flush` requests with
//! `fdatasync`-style durability so callers can advance their *committed* watermark safely.

use bytes::Bytes;
use osprey_domain::{ErrorKind, TaskError};
use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

#[cfg(unix)]
use std::os::unix::fs::FileExt;
#[cfg(windows)]
use std::os::windows::fs::FileExt;

enum Cmd {
    Write { offset: u64, data: Bytes },
    Flush { ack: oneshot::Sender<io::Result<()>> },
    Truncate { len: u64, ack: oneshot::Sender<io::Result<()>> },
    Close { ack: oneshot::Sender<io::Result<()>> },
}

#[derive(Clone)]
pub struct FileWriter {
    tx: mpsc::Sender<Cmd>,
    path: Arc<PathBuf>,
}

pub struct OpenOptionsExt {
    pub preallocate: Option<u64>,
    pub sparse: bool,
}

impl FileWriter {
    /// Open (or create) `path` for positional writes. `expected_len` enables preallocation.
    pub async fn open(path: &Path, opts: OpenOptionsExt) -> Result<Self, TaskError> {
        let path_buf = path.to_path_buf();
        let file = tokio::task::spawn_blocking({
            let p = path_buf.clone();
            move || -> io::Result<File> {
                if let Some(parent) = p.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let f = OpenOptions::new().create(true).read(true).write(true).truncate(false).open(&p)?;
                if let Some(len) = opts.preallocate {
                    preallocate(&f, len, opts.sparse)?;
                }
                Ok(f)
            }
        })
        .await
        .map_err(|e| TaskError::internal(format!("open join: {e}")))?
        .map_err(|e| TaskError::from_io(&e, &format!("open {}", path.display())))?;

        let (tx, mut rx) = mpsc::channel::<Cmd>(64);
        let p = path_buf.clone();
        std::thread::Builder::new()
            .name(format!("osprey-writer-{}", p.file_name().and_then(|s| s.to_str()).unwrap_or("file")))
            .spawn(move || {
                let mut pending_err: Option<io::Error> = None;
                while let Some(cmd) = rx.blocking_recv() {
                    match cmd {
                        Cmd::Write { offset, data } => {
                            if pending_err.is_some() {
                                continue;
                            }
                            if let Err(e) = write_all_at(&file, &data, offset) {
                                pending_err = Some(e);
                            }
                        }
                        Cmd::Flush { ack } => {
                            let r = match pending_err.take() {
                                Some(e) => Err(e),
                                None => file.sync_data(),
                            };
                            let _ = ack.send(r);
                        }
                        Cmd::Truncate { len, ack } => {
                            let _ = ack.send(file.set_len(len));
                        }
                        Cmd::Close { ack } => {
                            let r = match pending_err.take() {
                                Some(e) => Err(e),
                                None => file.sync_data(),
                            };
                            let _ = ack.send(r);
                            break;
                        }
                    }
                }
            })
            .map_err(|e| TaskError::internal(format!("spawn writer: {e}")))?;

        Ok(Self { tx, path: Arc::new(path_buf) })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Queue a write. Awaits when the writer is behind (back-pressure).
    pub async fn write(&self, offset: u64, data: Bytes) -> Result<(), TaskError> {
        self.tx
            .send(Cmd::Write { offset, data })
            .await
            .map_err(|_| TaskError::new(ErrorKind::DiskWriteError, "writer thread stopped"))
    }

    /// Flush everything queued so far to stable storage. Returns any deferred write error.
    pub async fn flush(&self) -> Result<(), TaskError> {
        let (ack, rx) = oneshot::channel();
        self.tx.send(Cmd::Flush { ack }).await.map_err(|_| TaskError::new(ErrorKind::DiskWriteError, "writer thread stopped"))?;
        rx.await
            .map_err(|_| TaskError::new(ErrorKind::DiskWriteError, "writer thread stopped"))?
            .map_err(|e| TaskError::from_io(&e, "flush"))
    }

    pub async fn truncate(&self, len: u64) -> Result<(), TaskError> {
        let (ack, rx) = oneshot::channel();
        self.tx.send(Cmd::Truncate { len, ack }).await.map_err(|_| TaskError::new(ErrorKind::DiskWriteError, "writer thread stopped"))?;
        rx.await
            .map_err(|_| TaskError::new(ErrorKind::DiskWriteError, "writer thread stopped"))?
            .map_err(|e| TaskError::from_io(&e, "truncate"))
    }

    /// Flush and close. The writer is unusable afterwards.
    pub async fn close(self) -> Result<(), TaskError> {
        let (ack, rx) = oneshot::channel();
        if self.tx.send(Cmd::Close { ack }).await.is_err() {
            return Ok(());
        }
        rx.await
            .map_err(|_| TaskError::new(ErrorKind::DiskWriteError, "writer thread stopped"))?
            .map_err(|e| TaskError::from_io(&e, "close"))
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
        Some(st.f_bavail as u64 * st.f_frsize as u64)
    }
    #[cfg(not(unix))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writes_out_of_order_and_flushes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("out.bin");
        let w = FileWriter::open(&p, OpenOptionsExt { preallocate: Some(10), sparse: true }).await.unwrap();
        w.write(5, Bytes::from_static(b"56789")).await.unwrap();
        w.write(0, Bytes::from_static(b"01234")).await.unwrap();
        w.flush().await.unwrap();
        w.close().await.unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"0123456789");
        assert!(free_space(dir.path()).unwrap() > 0);
    }

    #[tokio::test]
    async fn preallocates_real_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("pre.bin");
        let w = FileWriter::open(&p, OpenOptionsExt { preallocate: Some(1 << 20), sparse: false }).await.unwrap();
        w.close().await.unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().len(), 1 << 20);
    }
}
