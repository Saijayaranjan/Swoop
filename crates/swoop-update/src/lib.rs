//! Secure self-updates from GitHub Releases.
//!
//! * **Check**: `GET https://api.github.com/repos/<GITHUB_REPOSITORY>/releases/latest` (or the
//!   release list when beta releases are wanted) with `User-Agent: Swoop/<version>` and a cached
//!   `ETag`. A release is offered when its `vX.Y.Z` tag is strictly newer than the running version
//!   and it carries `Swoop-X.Y.Z.dmg` plus `Swoop-X.Y.Z.dmg.sig`. No releases (404), offline and
//!   rate-limited are ordinary [`UpdateStatus`]es, never errors.
//! * **Download**: HTTPS only, redirects included ([`http`]). The `.sig` asset is a base64 Ed25519
//!   signature over the DMG's SHA-256; the DMG is hashed while it streams, the signature checked
//!   against the release key compiled into the binary, and only then renamed into place.
//! * **Install** (macOS app only, [`install`]): re-verify, mount read-only, validate the bundle,
//!   stage it next to the running app, then a detached helper swaps bundles and relaunches.
//!
//! The release public key defaults to [`DEFAULT_PUBLIC_KEY_HEX`]; a build may override it with
//! the `SWOOP_UPDATE_PUBLIC_KEY` environment variable at compile time (never at run time).

pub mod github;
pub mod http;
#[cfg(unix)]
pub mod install;
pub mod signing;
pub mod version;

pub use github::{releases_api_base, releases_page, GITHUB_REPOSITORY};

use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::path::{Path, PathBuf};
use swoop_domain::{ErrorKind, Millis, TaskError};

/// The Ed25519 public key (hex) that release DMGs are signed with. Its private half lives with the
/// release manager (`~/.config/swoop/update-signing-key`), never in this repository.
pub const DEFAULT_PUBLIC_KEY_HEX: &str =
    "aa7535585959f3c6f8360e68d2c4e1a00529b47bdad3cbb7704752c01703a197";

/// The release key this build trusts: `SWOOP_UPDATE_PUBLIC_KEY` at compile time, else the default.
pub fn release_public_key_hex() -> &'static str {
    match option_env!("SWOOP_UPDATE_PUBLIC_KEY") {
        Some(k) if !k.trim().is_empty() => k,
        _ => DEFAULT_PUBLIC_KEY_HEX,
    }
}

/// Debug builds only: point the checker at another releases URL (a local test server).
pub const FEED_URL_ENV: &str = "SWOOP_UPDATE_FEED_URL";

/// The releases URL override from [`FEED_URL_ENV`]. Always `None` in release builds, so a shipped
/// app only ever asks GitHub.
pub fn feed_url_override() -> Option<String> {
    if cfg!(debug_assertions) {
        std::env::var(FEED_URL_ENV)
            .ok()
            .filter(|s| !s.trim().is_empty())
    } else {
        None
    }
}

/// Largest DMG the updater accepts.
const MAX_DMG_BYTES: u64 = 1024 * 1024 * 1024;
/// Largest API response / `.sig` asset read into memory.
const MAX_JSON_BYTES: usize = 4 * 1024 * 1024;
const MAX_SIG_BYTES: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateStatus {
    /// Nothing newer than the running version.
    #[default]
    UpToDate,
    /// A newer, correctly packaged release exists.
    Available,
    /// The repository has no published releases yet (GitHub answered 404).
    NoReleases,
    /// The network or GitHub couldn't be reached.
    Offline,
    /// GitHub's API rate limit is exhausted; `retry_at` says when it resets.
    RateLimited,
    /// Anything else (unexpected status, unparsable response).
    Error,
}

impl UpdateStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UpToDate => "up_to_date",
            Self::Available => "available",
            Self::NoReleases => "no_releases",
            Self::Offline => "offline",
            Self::RateLimited => "rate_limited",
            Self::Error => "error",
        }
    }
}

/// Result of a check.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateInfo {
    pub current_version: String,
    pub status: UpdateStatus,
    pub available: bool,
    pub latest_version: Option<String>,
    pub tag: Option<String>,
    /// Release title.
    pub name: Option<String>,
    /// Release body (Markdown).
    pub notes: Option<String>,
    /// The release's page on GitHub.
    pub notes_url: Option<String>,
    /// RFC 3339.
    pub published_at: Option<String>,
    pub prerelease: bool,
    pub download_url: Option<String>,
    pub size: Option<u64>,
    pub signature_url: Option<String>,
    /// Human-readable detail for the non-success statuses.
    pub message: Option<String>,
    /// When a rate limit resets.
    pub retry_at: Option<Millis>,
    pub checked_at: Millis,
}

/// A downloaded DMG whose signature checked out.
#[derive(Clone, Debug, PartialEq)]
pub struct VerifiedUpdate {
    pub path: PathBuf,
    pub version: semver::Version,
    /// The `.sig` asset's contents, kept to re-verify right before mounting.
    pub signature_b64: String,
    pub sha256_hex: String,
}

pub struct CheckerOptions {
    /// The running version (`0.1.0`).
    pub current_version: String,
    /// Hex Ed25519 public key; normally [`release_public_key_hex`].
    pub public_key_hex: String,
    /// Releases URL override (tests / debug builds); `None` = GitHub.
    pub releases_url: Option<String>,
    /// Where to keep the ETag + last response between checks.
    pub cache_path: Option<PathBuf>,
    pub proxy_url: Option<String>,
    /// Allow plain HTTP to loopback (tests / debug builds only).
    pub allow_local_http: bool,
}

impl CheckerOptions {
    /// Production defaults for `current_version`: GitHub, the compiled-in key, and in debug builds
    /// the [`FEED_URL_ENV`] override with loopback HTTP allowed.
    pub fn for_version(current_version: &str) -> Self {
        let releases_url = feed_url_override();
        Self {
            current_version: current_version.to_owned(),
            public_key_hex: release_public_key_hex().to_owned(),
            allow_local_http: cfg!(debug_assertions) && releases_url.is_some(),
            releases_url,
            cache_path: None,
            proxy_url: None,
        }
    }
}

pub struct UpdateChecker {
    http: reqwest::Client,
    key: VerifyingKey,
    current: semver::Version,
    releases_url: Option<String>,
    cache_path: Option<PathBuf>,
    allow_local_http: bool,
}

/// ETag cache entry.
#[derive(Serialize, Deserialize)]
struct CachedResponse {
    url: String,
    etag: String,
    body: String,
}

enum FetchError {
    NotFound,
    Offline(String),
    RateLimited(Option<Millis>, String),
    Other(String),
}

impl UpdateChecker {
    pub fn new(opts: CheckerOptions) -> Result<Self, TaskError> {
        let key = signing::parse_public_key(&opts.public_key_hex)?;
        let current = version::parse_version(&opts.current_version).ok_or_else(|| {
            TaskError::new(
                ErrorKind::Internal,
                format!("bad version {}", opts.current_version),
            )
        })?;
        let http = http::build_client(
            &format!("Swoop/{}", opts.current_version),
            opts.allow_local_http,
            opts.proxy_url.as_deref(),
        )?;
        Ok(Self {
            http,
            key,
            current,
            releases_url: opts.releases_url,
            cache_path: opts.cache_path,
            allow_local_http: opts.allow_local_http,
        })
    }

    pub fn current_version(&self) -> &semver::Version {
        &self.current
    }

    pub fn key(&self) -> &VerifyingKey {
        &self.key
    }

    fn releases_url(&self, include_prereleases: bool) -> String {
        if let Some(u) = &self.releases_url {
            return u.clone();
        }
        if include_prereleases {
            format!("{}?per_page=20", releases_api_base())
        } else {
            format!("{}/latest", releases_api_base())
        }
    }

    /// Ask for the newest release. Never fails: problems come back as a status + message.
    pub async fn check(&self, include_prereleases: bool) -> UpdateInfo {
        let mut info = UpdateInfo {
            current_version: self.current.to_string(),
            checked_at: Millis::now(),
            ..Default::default()
        };
        let url = self.releases_url(include_prereleases);
        let releases = match self.fetch_releases(&url).await {
            Ok(r) => r,
            Err(e) => {
                let (status, msg) = match e {
                    FetchError::NotFound => (
                        UpdateStatus::NoReleases,
                        "No releases have been published yet.".to_owned(),
                    ),
                    FetchError::Offline(m) => (UpdateStatus::Offline, m),
                    FetchError::RateLimited(at, m) => {
                        info.retry_at = at;
                        (UpdateStatus::RateLimited, m)
                    }
                    FetchError::Other(m) => (UpdateStatus::Error, m),
                };
                tracing::info!(status = status.as_str(), %msg, "update check");
                info.status = status;
                info.message = Some(msg);
                return info;
            }
        };
        if releases.is_empty() {
            info.status = UpdateStatus::NoReleases;
            info.message = Some("No releases have been published yet.".into());
            return info;
        }
        match github::select_release(&releases, include_prereleases, &self.current) {
            Some(c) => {
                info.status = UpdateStatus::Available;
                info.available = true;
                info.latest_version = Some(c.version.to_string());
                info.tag = Some(c.release.tag_name.clone());
                info.name = c.release.name.clone();
                info.notes = c.release.body.clone();
                info.notes_url = c.release.html_url.clone();
                info.published_at = c.release.published_at.clone();
                info.prerelease = c.release.prerelease || !c.version.pre.is_empty();
                info.download_url = Some(c.dmg.browser_download_url.clone());
                info.size = (c.dmg.size > 0).then_some(c.dmg.size);
                info.signature_url = Some(c.sig.browser_download_url.clone());
            }
            None => info.status = UpdateStatus::UpToDate,
        }
        info
    }

    async fn fetch_releases(&self, url: &str) -> Result<Vec<github::GhRelease>, FetchError> {
        let parsed = http::require_allowed(url, self.allow_local_http)
            .map_err(|e| FetchError::Other(e.message))?;
        let cached = self.load_cache(url);
        let mut req = self
            .http
            .get(parsed)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(c) = &cached {
            req = req.header("If-None-Match", &c.etag);
        }
        let resp = req.send().await.map_err(|e| {
            if e.is_redirect() {
                FetchError::Other(e.to_string())
            } else {
                FetchError::Offline(format!("Couldn't reach GitHub: {}", root_cause(&e)))
            }
        })?;
        let status = resp.status().as_u16();
        let header = |name: &str| {
            resp.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        match status {
            304 => {
                let body = cached.map(|c| c.body).ok_or_else(|| {
                    FetchError::Other("GitHub answered 304 without a cached response".into())
                })?;
                github::parse_releases(body.as_bytes())
                    .map_err(|e| FetchError::Other(format!("cached release data: {e}")))
            }
            404 => Err(FetchError::NotFound),
            403 | 429 => {
                let remaining = header("x-ratelimit-remaining");
                let retry_after = header("retry-after").and_then(|v| v.parse::<i64>().ok());
                if remaining.as_deref() == Some("0") || retry_after.is_some() || status == 429 {
                    let reset = header("x-ratelimit-reset")
                        .and_then(|v| v.parse::<i64>().ok())
                        .map(|secs| Millis(secs * 1000))
                        .or_else(|| retry_after.map(|s| Millis(Millis::now().0 + s * 1000)));
                    Err(FetchError::RateLimited(
                        reset,
                        "GitHub's rate limit was reached; Swoop will try again later.".into(),
                    ))
                } else {
                    Err(FetchError::Other(format!(
                        "GitHub refused the request (HTTP {status})"
                    )))
                }
            }
            200..=299 => {
                let etag = header("etag");
                let body = read_limited(resp, MAX_JSON_BYTES).await.map_err(|e| {
                    FetchError::Offline(format!("Couldn't read GitHub's reply: {e}"))
                })?;
                let releases = github::parse_releases(&body)
                    .map_err(|e| FetchError::Other(format!("unexpected release data: {e}")))?;
                if let Some(etag) = etag {
                    self.store_cache(url, &etag, &body);
                }
                Ok(releases)
            }
            500..=599 => Err(FetchError::Offline(format!(
                "GitHub is having trouble (HTTP {status})"
            ))),
            _ => Err(FetchError::Other(format!("GitHub answered HTTP {status}"))),
        }
    }

    fn load_cache(&self, url: &str) -> Option<CachedResponse> {
        let p = self.cache_path.as_ref()?;
        let c: CachedResponse = serde_json::from_slice(&std::fs::read(p).ok()?).ok()?;
        (c.url == url).then_some(c)
    }

    fn store_cache(&self, url: &str, etag: &str, body: &[u8]) {
        let Some(p) = &self.cache_path else { return };
        let entry = CachedResponse {
            url: url.to_owned(),
            etag: etag.to_owned(),
            body: String::from_utf8_lossy(body).into_owned(),
        };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(bytes) = serde_json::to_vec(&entry) {
            let _ = std::fs::write(p, bytes);
        }
    }

    /// Fetch the `.sig`, stream the DMG while hashing it, and check the signature. The DMG only
    /// lands at `dest_dir/Swoop-X.Y.Z.dmg` once it verified; any failure deletes it.
    /// `progress(received, total)` is called as bytes arrive.
    pub async fn download_and_verify(
        &self,
        info: &UpdateInfo,
        dest_dir: &Path,
        progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
    ) -> Result<VerifiedUpdate, TaskError> {
        let (Some(dmg_url), Some(sig_url), Some(v)) = (
            info.download_url.as_deref(),
            info.signature_url.as_deref(),
            info.latest_version.as_deref(),
        ) else {
            return Err(TaskError::new(ErrorKind::Internal, "no update available"));
        };
        let version = version::parse_version(v)
            .ok_or_else(|| TaskError::new(ErrorKind::ParseError, format!("bad version {v}")))?;
        if !version::is_newer(&version, &self.current) {
            return Err(TaskError::new(ErrorKind::Forbidden, "refusing a downgrade"));
        }
        let dmg_url = http::require_allowed(dmg_url, self.allow_local_http)?;
        let sig_url = http::require_allowed(sig_url, self.allow_local_http)?;

        // 1. the signature (tiny)
        let resp = self.get_asset(sig_url).await?;
        let sig_bytes = read_limited(resp, MAX_SIG_BYTES)
            .await
            .map_err(|e| TaskError::new(ErrorKind::Truncated, e))?;
        let sig_text = String::from_utf8_lossy(&sig_bytes).trim().to_owned();
        let signature = signing::parse_signature(&sig_text)?;

        // 2. the DMG, hashed as it streams, under a temporary name
        tokio::fs::create_dir_all(dest_dir)
            .await
            .map_err(|e| TaskError::from_io(&e, "create update dir"))?;
        let final_path = dest_dir.join(github::dmg_asset_name(&version));
        let tmp = dest_dir.join(format!("{}.part", github::dmg_asset_name(&version)));
        let resp = self.get_asset(dmg_url).await?;
        let expected = info.size.or(resp.content_length());
        let digest = match stream_to(resp, &tmp, expected, progress).await {
            Ok(d) => d,
            Err(e) => {
                let _ = tokio::fs::remove_file(&tmp).await;
                return Err(e);
            }
        };

        // 3. verify before anything can look at it
        if let Err(e) = signing::verify_digest(&self.key, &digest, &signature) {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(e);
        }
        tokio::fs::rename(&tmp, &final_path)
            .await
            .map_err(|e| TaskError::from_io(&e, "rename verified update"))?;
        Ok(VerifiedUpdate {
            path: final_path,
            version,
            signature_b64: sig_text,
            sha256_hex: hex::encode(digest),
        })
    }

    async fn get_asset(&self, url: url::Url) -> Result<reqwest::Response, TaskError> {
        let resp = self
            .http
            .get(url.clone())
            .header("Accept", "application/octet-stream")
            .send()
            .await
            .map_err(|e| {
                let kind = if e.is_redirect() {
                    ErrorKind::Forbidden
                } else {
                    ErrorKind::ConnectionReset
                };
                TaskError::new(kind, root_cause(&e))
            })?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(TaskError::from_http_status(status, url.as_str()));
        }
        Ok(resp)
    }
}

async fn read_limited(mut resp: reqwest::Response, max: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        if out.len() + chunk.len() > max {
            return Err("response too large".into());
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

async fn stream_to(
    mut resp: reqwest::Response,
    path: &Path,
    expected: Option<u64>,
    progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
) -> Result<[u8; 32], TaskError> {
    use tokio::io::AsyncWriteExt;
    let cap = expected.unwrap_or(MAX_DMG_BYTES).min(MAX_DMG_BYTES);
    let mut file = tokio::fs::File::create(path)
        .await
        .map_err(|e| TaskError::from_io(&e, "create"))?;
    let mut hasher = sha2::Sha256::new();
    let mut total = 0u64;
    progress(0, expected);
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| TaskError::new(ErrorKind::Truncated, e.to_string()))?
    {
        total += chunk.len() as u64;
        if total > cap {
            return Err(TaskError::new(
                ErrorKind::ChecksumMismatch,
                "the update is larger than announced",
            ));
        }
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|e| TaskError::from_io(&e, "write"))?;
        progress(total, expected);
    }
    file.sync_all()
        .await
        .map_err(|e| TaskError::from_io(&e, "sync"))?;
    if let Some(n) = expected {
        if n != total {
            return Err(TaskError::new(
                ErrorKind::Truncated,
                format!("the update download was cut short ({total} of {n} bytes)"),
            ));
        }
    }
    Ok(hasher.finalize().into())
}

fn root_cause(e: &(dyn std::error::Error + 'static)) -> String {
    let mut cur = e;
    while let Some(next) = cur.source() {
        cur = next;
    }
    cur.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use swoop_testserver::TestServer;

    fn release_json(server: &TestServer, v: &str) -> String {
        serde_json::json!({
            "tag_name": format!("v{v}"), "name": format!("Swoop {v}"), "draft": false, "prerelease": false,
            "published_at": "2026-10-01T09:30:00Z", "html_url": "https://github.com/Saijayaranjan/Swoop/releases/tag/v9.9.9",
            "body": "## Notes\n- one",
            "assets": [
                { "name": format!("Swoop-{v}.dmg"), "size": 50_000, "browser_download_url": server.url(&format!("/bytes/Swoop-{v}.dmg")) },
                { "name": format!("Swoop-{v}.dmg.sig"), "size": 88, "browser_download_url": server.url(&format!("/text/Swoop-{v}.dmg.sig")) }
            ]
        })
        .to_string()
    }

    fn checker(server: &TestServer, key_hex: &str, current: &str, feed: &str) -> UpdateChecker {
        UpdateChecker::new(CheckerOptions {
            current_version: current.into(),
            public_key_hex: key_hex.into(),
            releases_url: Some(server.url(feed)),
            cache_path: None,
            proxy_url: None,
            allow_local_http: true,
        })
        .unwrap()
    }

    #[tokio::test]
    async fn check_download_verify_and_tamper() {
        let server = TestServer::start().await;
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let pk_hex = signing::public_key_hex(&sk);
        let dmg = swoop_testserver::content_for("update", 50_000);
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.dmg");
        std::fs::write(&src, &dmg).unwrap();
        let sig = signing::sign_file(&sk, &src).unwrap();
        server.add_bytes("Swoop-9.9.9.dmg", "application/octet-stream", dmg.clone());
        server.add_text("Swoop-9.9.9.dmg.sig", "text/plain", &format!("{sig}\n"));
        server.add_text(
            "latest.json",
            "application/json",
            &release_json(&server, "9.9.9"),
        );

        let c = checker(&server, &pk_hex, "1.0.0", "/text/latest.json");
        let info = c.check(false).await;
        assert_eq!(info.status, UpdateStatus::Available);
        assert_eq!(info.latest_version.as_deref(), Some("9.9.9"));
        assert_eq!(info.notes.as_deref(), Some("## Notes\n- one"));
        let out = dir.path().join("out");
        let seen = std::sync::atomic::AtomicU64::new(0);
        let v = c
            .download_and_verify(&info, &out, &|n, _| {
                seen.store(n, std::sync::atomic::Ordering::Relaxed)
            })
            .await
            .unwrap();
        assert_eq!(std::fs::read(&v.path).unwrap(), dmg);
        assert_eq!(v.path.file_name().unwrap(), "Swoop-9.9.9.dmg");
        assert_eq!(seen.load(std::sync::atomic::Ordering::Relaxed), 50_000);

        // A tampered DMG (same size, one byte flipped) is refused and never left on disk.
        let mut bad = dmg.to_vec();
        bad[1234] ^= 0xff;
        server.add_bytes("Swoop-9.9.9.dmg", "application/octet-stream", bad.into());
        let out2 = dir.path().join("out2");
        let err = c
            .download_and_verify(&info, &out2, &|_, _| {})
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::CertificateInvalid);
        assert_eq!(std::fs::read_dir(&out2).unwrap().count(), 0);

        // Signed by some other key → refused.
        server.add_bytes("Swoop-9.9.9.dmg", "application/octet-stream", dmg.clone());
        let other = SigningKey::from_bytes(&[9u8; 32]);
        let other_c = checker(
            &server,
            &signing::public_key_hex(&other),
            "1.0.0",
            "/text/latest.json",
        );
        assert!(other_c
            .download_and_verify(&info, &out2, &|_, _| {})
            .await
            .is_err());

        // Already on 9.9.9 → up to date, and a download is refused as a downgrade.
        let newer = checker(&server, &pk_hex, "9.9.9", "/text/latest.json");
        assert_eq!(newer.check(false).await.status, UpdateStatus::UpToDate);
        assert!(newer
            .download_and_verify(&info, &out2, &|_, _| {})
            .await
            .is_err());
    }

    #[tokio::test]
    async fn quiet_statuses() {
        let server = TestServer::start().await;
        let pk = signing::public_key_hex(&SigningKey::from_bytes(&[7u8; 32]));
        // 404: no releases yet
        let c = checker(&server, &pk, "0.1.0", "/file/latest?status=404");
        assert_eq!(c.check(false).await.status, UpdateStatus::NoReleases);
        // offline: nothing listening
        let dead = UpdateChecker::new(CheckerOptions {
            releases_url: Some("http://127.0.0.1:9/releases/latest".into()),
            allow_local_http: true,
            ..CheckerOptions::for_version("0.1.0")
        })
        .unwrap();
        assert_eq!(dead.check(false).await.status, UpdateStatus::Offline);
        // rate limited
        let c = checker(&server, &pk, "0.1.0", "/file/latest?status=429");
        assert_eq!(c.check(false).await.status, UpdateStatus::RateLimited);
    }
}
