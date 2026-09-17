//! Streaming checksum computation for MD5, SHA-1, SHA-256, SHA-512 and BLAKE3.

use osprey_domain::{Checksum, ChecksumAlgorithm, ErrorKind, TaskError};
use sha1::Digest as _;
use std::path::Path;

pub enum Hasher {
    Md5(md5::Md5),
    Sha1(sha1::Sha1),
    Sha256(sha2::Sha256),
    Sha512(sha2::Sha512),
    Blake3(Box<blake3::Hasher>),
}

impl Hasher {
    pub fn new(algorithm: ChecksumAlgorithm) -> Self {
        match algorithm {
            ChecksumAlgorithm::Md5 => Hasher::Md5(md5::Md5::new()),
            ChecksumAlgorithm::Sha1 => Hasher::Sha1(sha1::Sha1::new()),
            ChecksumAlgorithm::Sha256 => Hasher::Sha256(sha2::Sha256::new()),
            ChecksumAlgorithm::Sha512 => Hasher::Sha512(sha2::Sha512::new()),
            ChecksumAlgorithm::Blake3 => Hasher::Blake3(Box::new(blake3::Hasher::new())),
        }
    }
    pub fn update(&mut self, data: &[u8]) {
        match self {
            Hasher::Md5(h) => h.update(data),
            Hasher::Sha1(h) => h.update(data),
            Hasher::Sha256(h) => h.update(data),
            Hasher::Sha512(h) => h.update(data),
            Hasher::Blake3(h) => {
                h.update(data);
            }
        }
    }
    pub fn finalize_hex(self) -> String {
        match self {
            Hasher::Md5(h) => hex::encode(h.finalize()),
            Hasher::Sha1(h) => hex::encode(h.finalize()),
            Hasher::Sha256(h) => hex::encode(h.finalize()),
            Hasher::Sha512(h) => hex::encode(h.finalize()),
            Hasher::Blake3(h) => h.finalize().to_hex().to_string(),
        }
    }
    pub fn algorithm(&self) -> ChecksumAlgorithm {
        match self {
            Hasher::Md5(_) => ChecksumAlgorithm::Md5,
            Hasher::Sha1(_) => ChecksumAlgorithm::Sha1,
            Hasher::Sha256(_) => ChecksumAlgorithm::Sha256,
            Hasher::Sha512(_) => ChecksumAlgorithm::Sha512,
            Hasher::Blake3(_) => ChecksumAlgorithm::Blake3,
        }
    }
}

/// Hash a file on a blocking thread, reporting progress through `on_progress(bytes_done)`.
pub async fn hash_file(
    path: &Path,
    algorithm: ChecksumAlgorithm,
    cancel: osprey_domain::engine::tokio_util_lite::CancellationToken,
    on_progress: impl Fn(u64) + Send + 'static,
) -> Result<Checksum, TaskError> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let mut f = std::fs::File::open(&path).map_err(|e| TaskError::from_io(&e, "open for checksum"))?;
        let mut hasher = Hasher::new(algorithm);
        let mut buf = vec![0u8; 1024 * 1024];
        let mut done = 0u64;
        let mut last_report = 0u64;
        loop {
            if cancel.is_cancelled() {
                return Err(TaskError::cancelled());
            }
            let n = f.read(&mut buf).map_err(|e| TaskError::from_io(&e, "read for checksum"))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            done += n as u64;
            if done - last_report >= 8 * 1024 * 1024 {
                on_progress(done);
                last_report = done;
            }
        }
        on_progress(done);
        Ok(Checksum::new(algorithm, hasher.finalize_hex()))
    })
    .await
    .map_err(|e| TaskError::new(ErrorKind::Internal, format!("checksum task failed: {e}")))?
}

/// Verify a file against an expected checksum.
pub async fn verify_file(path: &Path, expected: &Checksum, cancel: osprey_domain::engine::tokio_util_lite::CancellationToken) -> Result<Checksum, TaskError> {
    let actual = hash_file(path, expected.algorithm, cancel, |_| {}).await?;
    if actual.value == expected.value.to_ascii_lowercase() {
        Ok(actual)
    } else {
        Err(TaskError::new(ErrorKind::ChecksumMismatch, format!("{} mismatch", expected.algorithm.as_str()))
            .with_detail(format!("expected {} got {}", expected.value, actual.value)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hashes_known_vectors() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        std::fs::write(&p, b"abc").unwrap();
        let c = osprey_domain::engine::tokio_util_lite::CancellationToken::new();
        let sha = hash_file(&p, ChecksumAlgorithm::Sha256, c.clone(), |_| {}).await.unwrap();
        assert_eq!(sha.value, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let md5 = hash_file(&p, ChecksumAlgorithm::Md5, c.clone(), |_| {}).await.unwrap();
        assert_eq!(md5.value, "900150983cd24fb0d6963f7d28e17f72");
        let sha1 = hash_file(&p, ChecksumAlgorithm::Sha1, c.clone(), |_| {}).await.unwrap();
        assert_eq!(sha1.value, "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert!(verify_file(&p, &Checksum::new(ChecksumAlgorithm::Sha256, "00"), c).await.is_err());
    }
}
