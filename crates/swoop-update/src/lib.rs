//! Secure updates: a JSON feed lists releases with an Ed25519 signature over the archive's
//! SHA-256; the archive is downloaded, hashed, and the signature verified before anything is
//! extracted or installed. Downgrades are refused.
//!
//! Feed format (`https://…/appcast.json`):
//! ```json
//! { "releases": [ { "version": "1.2.0", "channel": "stable", "notes": "…", "notes_url": "…",
//!   "artifacts": { "macos-arm64": { "url": "…dmg", "size": 123, "sha256": "…", "signature": "<base64 ed25519 over the 32 sha256 bytes>" } } } ] }
//! ```

use base64::Engine as _;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use swoop_domain::{ErrorKind, Millis, TaskError};
use swoop_runtime::net::ClientFactory;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub current_version: String,
    pub available: bool,
    pub latest_version: Option<String>,
    pub notes: Option<String>,
    pub notes_url: Option<String>,
    pub download_url: Option<String>,
    pub size: Option<u64>,
    pub sha256: Option<String>,
    pub signature_b64: Option<String>,
    /// Set after `download_and_verify`.
    pub signature_valid: Option<bool>,
    pub checked_at: Millis,
}

#[derive(Deserialize)]
struct Feed {
    releases: Vec<Release>,
}
#[derive(Deserialize)]
struct Release {
    version: String,
    #[serde(default = "default_channel")]
    channel: String,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    notes_url: Option<String>,
    #[serde(default)]
    artifacts: std::collections::BTreeMap<String, Artifact>,
}
fn default_channel() -> String {
    "stable".into()
}
#[derive(Deserialize, Clone)]
struct Artifact {
    url: String,
    #[serde(default)]
    size: Option<u64>,
    sha256: String,
    signature: String,
}

pub fn platform_key() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "macos-arm64",
        ("macos", _) => "macos-x86_64",
        ("windows", _) => "windows-x86_64",
        ("linux", "aarch64") => "linux-arm64",
        _ => "linux-x86_64",
    }
}

pub struct UpdateChecker {
    clients: Arc<ClientFactory>,
    feed_url: String,
    key: VerifyingKey,
    current: semver::Version,
}

impl UpdateChecker {
    pub fn new(
        clients: Arc<ClientFactory>,
        feed_url: String,
        public_key_hex: &str,
        current_version: &str,
    ) -> Result<Self, TaskError> {
        let bytes = hex::decode(public_key_hex)
            .map_err(|e| TaskError::new(ErrorKind::Internal, format!("bad update key: {e}")))?;
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| TaskError::new(ErrorKind::Internal, "update key must be 32 bytes"))?;
        let key = VerifyingKey::from_bytes(&arr)
            .map_err(|e| TaskError::new(ErrorKind::Internal, format!("bad update key: {e}")))?;
        let current = semver::Version::parse(current_version)
            .map_err(|e| TaskError::new(ErrorKind::Internal, format!("bad version: {e}")))?;
        Ok(Self {
            clients,
            feed_url,
            key,
            current,
        })
    }

    pub async fn check(&self, channel: &str) -> Result<UpdateInfo, TaskError> {
        let url = url::Url::parse(&self.feed_url)
            .map_err(|e| TaskError::new(ErrorKind::InvalidUrl, e.to_string()))?;
        if url.scheme() != "https" && !swoop_runtime::net::is_local_url(&url) {
            return Err(TaskError::new(
                ErrorKind::Forbidden,
                "update feed must be https",
            ));
        }
        let client = self.clients.default_client()?;
        let resp = client
            .get(url.clone())
            .send()
            .await
            .map_err(|e| TaskError::new(ErrorKind::ConnectionReset, e.to_string()))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(TaskError::from_http_status(status, url.as_str()));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| TaskError::new(ErrorKind::Truncated, e.to_string()))?;
        if bytes.len() > 1024 * 1024 {
            return Err(TaskError::new(ErrorKind::ParseError, "feed too large"));
        }
        let feed: Feed = serde_json::from_slice(&bytes)
            .map_err(|e| TaskError::new(ErrorKind::ParseError, format!("feed: {e}")))?;
        let key = platform_key();
        let mut best: Option<(semver::Version, &Release, Artifact)> = None;
        for r in &feed.releases {
            if r.channel != channel {
                continue;
            }
            let Ok(v) = semver::Version::parse(&r.version) else {
                continue;
            };
            let Some(a) = r.artifacts.get(key) else {
                continue;
            };
            if v > self.current && best.as_ref().map(|(bv, _, _)| v > *bv).unwrap_or(true) {
                best = Some((v, r, a.clone()));
            }
        }
        let mut info = UpdateInfo {
            current_version: self.current.to_string(),
            checked_at: Millis::now(),
            ..Default::default()
        };
        if let Some((v, r, a)) = best {
            info.available = true;
            info.latest_version = Some(v.to_string());
            info.notes = r.notes.clone();
            info.notes_url = r.notes_url.clone();
            info.download_url = Some(a.url);
            info.size = a.size;
            info.sha256 = Some(a.sha256.to_ascii_lowercase());
            info.signature_b64 = Some(a.signature);
        }
        Ok(info)
    }

    /// Verify the signature over the declared SHA-256 first (cheap, catches a tampered feed),
    /// then download, hash and compare. Returns the verified archive path.
    pub async fn download_and_verify(
        &self,
        info: &UpdateInfo,
        dest_dir: &Path,
    ) -> Result<PathBuf, TaskError> {
        let (url, sha_hex, sig_b64, version) = match (
            &info.download_url,
            &info.sha256,
            &info.signature_b64,
            &info.latest_version,
        ) {
            (Some(u), Some(s), Some(g), Some(v)) => (u, s, g, v),
            _ => return Err(TaskError::new(ErrorKind::Internal, "no update available")),
        };
        let v = semver::Version::parse(version)
            .map_err(|e| TaskError::new(ErrorKind::ParseError, e.to_string()))?;
        if v <= self.current {
            return Err(TaskError::new(ErrorKind::Forbidden, "refusing downgrade"));
        }
        self.verify_signature(sha_hex, sig_b64)?;
        let parsed = url::Url::parse(url)
            .map_err(|e| TaskError::new(ErrorKind::InvalidUrl, e.to_string()))?;
        if parsed.scheme() != "https" && !swoop_runtime::net::is_local_url(&parsed) {
            return Err(TaskError::new(
                ErrorKind::Forbidden,
                "update artifact must be https",
            ));
        }
        tokio::fs::create_dir_all(dest_dir)
            .await
            .map_err(|e| TaskError::from_io(&e, "create update dir"))?;
        let name = swoop_runtime::filename::from_url(url)
            .unwrap_or_else(|| format!("swoop-{version}.bin"));
        let path = dest_dir.join(swoop_runtime::safety::sanitize_filename(&name));
        let client = self.clients.default_client()?;
        let mut resp = client
            .get(parsed.clone())
            .send()
            .await
            .map_err(|e| TaskError::new(ErrorKind::ConnectionReset, e.to_string()))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(TaskError::from_http_status(status, url));
        }
        // Write to a temporary name and only rename to the final name once the hash and size
        // match, so an interrupted or tampered download never sits where an installer looks.
        let tmp = path.with_extension("swoop-verify");
        let result = self.fetch_to(&mut resp, &tmp, sha_hex, info.size).await;
        if let Err(e) = result {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(e);
        }
        tokio::fs::rename(&tmp, &path)
            .await
            .map_err(|e| TaskError::from_io(&e, "rename verified update"))?;
        Ok(path)
    }

    async fn fetch_to(
        &self,
        resp: &mut reqwest::Response,
        path: &Path,
        sha_hex: &str,
        size: Option<u64>,
    ) -> Result<(), TaskError> {
        let mut file = tokio::fs::File::create(path)
            .await
            .map_err(|e| TaskError::from_io(&e, "create"))?;
        let mut hasher = sha2::Sha256::new();
        let mut total = 0u64;
        use tokio::io::AsyncWriteExt;
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| TaskError::new(ErrorKind::Truncated, e.to_string()))?
        {
            hasher.update(&chunk);
            file.write_all(&chunk)
                .await
                .map_err(|e| TaskError::from_io(&e, "write"))?;
            total += chunk.len() as u64;
            if total > 2 * 1024 * 1024 * 1024 {
                return Err(TaskError::new(
                    ErrorKind::ParseError,
                    "update larger than 2 GiB",
                ));
            }
        }
        file.sync_all()
            .await
            .map_err(|e| TaskError::from_io(&e, "sync"))?;
        let actual = hex::encode(hasher.finalize());
        if actual != sha_hex.to_ascii_lowercase() {
            return Err(TaskError::new(
                ErrorKind::ChecksumMismatch,
                "update archive hash mismatch",
            )
            .with_detail(format!("expected {sha_hex} got {actual}")));
        }
        if let Some(sz) = size {
            if sz != total {
                return Err(TaskError::new(
                    ErrorKind::ChecksumMismatch,
                    "update size mismatch",
                ));
            }
        }
        Ok(())
    }

    fn verify_signature(&self, sha_hex: &str, sig_b64: &str) -> Result<(), TaskError> {
        let digest = hex::decode(sha_hex)
            .map_err(|_| TaskError::new(ErrorKind::ParseError, "bad sha256 in feed"))?;
        let sig_bytes = base64::engine::general_purpose::STANDARD
            .decode(sig_b64)
            .map_err(|_| TaskError::new(ErrorKind::ParseError, "bad signature encoding"))?;
        let sig = Signature::from_slice(&sig_bytes)
            .map_err(|_| TaskError::new(ErrorKind::ParseError, "bad signature length"))?;
        self.key.verify(&digest, &sig).map_err(|_| {
            TaskError::new(ErrorKind::CertificateInvalid, "update signature is invalid")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use swoop_domain::settings::Settings;

    #[tokio::test]
    async fn checks_and_verifies() {
        let server = swoop_testserver::TestServer::start().await;
        let artifact = swoop_testserver::content_for("update", 50_000);
        let sha = hex::encode(sha2::Sha256::digest(&artifact));
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let sig = base64::engine::general_purpose::STANDARD
            .encode(sk.sign(&hex::decode(&sha).unwrap()).to_bytes());
        let pk_hex = hex::encode(sk.verifying_key().to_bytes());
        server.add_bytes(
            "Swoop-9.9.9.dmg",
            "application/octet-stream",
            artifact.clone(),
        );
        let feed = serde_json::json!({ "releases": [
            { "version": "9.9.9", "channel": "stable", "notes": "Big", "artifacts": { platform_key(): { "url": server.url("/bytes/Swoop-9.9.9.dmg"), "size": 50000, "sha256": sha, "signature": sig } } },
            { "version": "0.0.1", "channel": "stable", "artifacts": { platform_key(): { "url": "x", "sha256": "00", "signature": "AA==" } } },
            { "version": "10.0.0", "channel": "beta", "artifacts": { platform_key(): { "url": "x", "sha256": "00", "signature": "AA==" } } }
        ]});
        server.add_text("appcast.json", "application/json", &feed.to_string());
        let clients = ClientFactory::new(Arc::new(Settings::default()));
        let c = UpdateChecker::new(
            clients.clone(),
            server.url("/text/appcast.json"),
            &pk_hex,
            "1.0.0",
        )
        .unwrap();
        let info = c.check("stable").await.unwrap();
        assert!(info.available);
        assert_eq!(info.latest_version.as_deref(), Some("9.9.9"));
        let dir = tempfile::tempdir().unwrap();
        let p = c.download_and_verify(&info, dir.path()).await.unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), artifact);
        // tampered signature → refused before download
        let mut bad = info.clone();
        bad.signature_b64 = Some(base64::engine::general_purpose::STANDARD.encode([1u8; 64]));
        assert_eq!(
            c.download_and_verify(&bad, dir.path())
                .await
                .unwrap_err()
                .kind,
            ErrorKind::CertificateInvalid
        );
        // wrong key → refused
        let other = UpdateChecker::new(
            clients.clone(),
            server.url("/text/appcast.json"),
            &hex::encode(
                SigningKey::from_bytes(&[9u8; 32])
                    .verifying_key()
                    .to_bytes(),
            ),
            "1.0.0",
        )
        .unwrap();
        assert!(other.download_and_verify(&info, dir.path()).await.is_err());
        // downgrade refused
        let newer =
            UpdateChecker::new(clients, server.url("/text/appcast.json"), &pk_hex, "20.0.0")
                .unwrap();
        assert!(!newer.check("stable").await.unwrap().available);
        assert!(newer.download_and_verify(&info, dir.path()).await.is_err());
    }
}
