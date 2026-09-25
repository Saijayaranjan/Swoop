//! Joins downloaded HLS segments into one file. MPEG-TS segments concatenate directly; fMP4
//! segments concatenate after the init segment. When `ffmpeg` is available a TS result is
//! remuxed (`-c copy`, no re-encode) into MP4 for better player compatibility.

use std::path::{Path, PathBuf};
use swoop_domain::{ErrorKind, TaskError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Concatenate `parts` (in order) into `out`, returning total bytes written.
pub async fn concatenate(parts: &[PathBuf], out: &Path) -> Result<u64, TaskError> {
    let mut dst = tokio::fs::File::create(out)
        .await
        .map_err(|e| TaskError::from_io(&e, "create merged file"))?;
    let mut total = 0u64;
    let mut buf = vec![0u8; 1024 * 1024];
    for p in parts {
        let mut src = tokio::fs::File::open(p)
            .await
            .map_err(|e| TaskError::from_io(&e, "open segment"))?;
        loop {
            let n = src
                .read(&mut buf)
                .await
                .map_err(|e| TaskError::from_io(&e, "read segment"))?;
            if n == 0 {
                break;
            }
            dst.write_all(&buf[..n])
                .await
                .map_err(|e| TaskError::from_io(&e, "write merged"))?;
            total += n as u64;
        }
    }
    dst.flush()
        .await
        .map_err(|e| TaskError::from_io(&e, "flush merged"))?;
    dst.sync_all()
        .await
        .map_err(|e| TaskError::from_io(&e, "sync merged"))?;
    Ok(total)
}

/// Locate an `ffmpeg` binary (PATH, Homebrew, MacPorts). Optional: absence is not an error.
pub fn find_ffmpeg() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SWOOP_FFMPEG") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    let candidates = [
        "/opt/homebrew/bin/ffmpeg",
        "/usr/local/bin/ffmpeg",
        "/opt/local/bin/ffmpeg",
        "/usr/bin/ffmpeg",
    ];
    for c in candidates {
        if Path::new(c).is_file() {
            return Some(PathBuf::from(c));
        }
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let p = Path::new(dir).join("ffmpeg");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Remux a TS file into MP4 without re-encoding. Returns `Ok(false)` if ffmpeg is unavailable
/// or refused the input (the caller keeps the TS file).
pub async fn remux_to_mp4(ffmpeg: &Path, input: &Path, output: &Path) -> Result<bool, TaskError> {
    let status = tokio::process::Command::new(ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(input)
        .args([
            "-c",
            "copy",
            "-movflags",
            "+faststart",
            "-bsf:a",
            "aac_adtstoasc",
        ])
        .arg(output)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map_err(|e| TaskError::new(ErrorKind::Internal, format!("ffmpeg: {e}")))?;
    if status.success()
        && tokio::fs::metadata(output)
            .await
            .map(|m| m.len() > 0)
            .unwrap_or(false)
    {
        Ok(true)
    } else {
        let _ = tokio::fs::remove_file(output).await;
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn concatenates_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::write(&a, b"hello ").unwrap();
        std::fs::write(&b, b"world").unwrap();
        let out = dir.path().join("out");
        let n = concatenate(&[a, b], &out).await.unwrap();
        assert_eq!(n, 11);
        assert_eq!(std::fs::read(&out).unwrap(), b"hello world");
    }
}
