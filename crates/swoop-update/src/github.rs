//! The GitHub Releases API: response parsing and picking the release to offer.
//!
//! Stable checks call `GET /repos/{owner}/{repo}/releases/latest`, which GitHub already limits to
//! the newest published, non-draft, non-prerelease release. Beta checks call
//! `GET /repos/{owner}/{repo}/releases?per_page=20` and consider prereleases too. Either way the
//! same filter runs here, so a response is never trusted to have done the filtering.

use crate::version::{is_newer, parse_tag};
use semver::Version;
use serde::Deserialize;

/// Owner and repository whose releases feed the updater (`owner/repo`). The one place to change.
pub const GITHUB_REPOSITORY: &str = "Saijayaranjan/Swoop";

/// `https://api.github.com/repos/<owner>/<repo>/releases`.
pub fn releases_api_base() -> String {
    format!("https://api.github.com/repos/{GITHUB_REPOSITORY}/releases")
}

/// `https://github.com/<owner>/<repo>/releases`, for "see all releases" links.
pub fn releases_page() -> String {
    format!("https://github.com/{GITHUB_REPOSITORY}/releases")
}

/// One release as the REST API returns it (only the fields the updater reads).
#[derive(Clone, Debug, Deserialize)]
pub struct GhRelease {
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub assets: Vec<GhAsset>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GhAsset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
}

/// Parse either `/releases/latest` (one object) or `/releases` (an array).
pub fn parse_releases(body: &[u8]) -> Result<Vec<GhRelease>, serde_json::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        Many(Vec<GhRelease>),
        One(Box<GhRelease>),
    }
    Ok(match serde_json::from_slice::<OneOrMany>(body)? {
        OneOrMany::Many(v) => v,
        OneOrMany::One(r) => vec![*r],
    })
}

/// `Swoop-1.2.3.dmg`: the installer asset a release must carry.
pub fn dmg_asset_name(v: &Version) -> String {
    format!("Swoop-{v}.dmg")
}

/// `Swoop-1.2.3.dmg.sig`: base64 Ed25519 signature over the DMG's SHA-256.
pub fn sig_asset_name(v: &Version) -> String {
    format!("{}.sig", dmg_asset_name(v))
}

/// A release worth offering, with its two assets located.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub version: Version,
    pub release: GhRelease,
    pub dmg: GhAsset,
    pub sig: GhAsset,
}

/// The newest release that is published (not a draft), allowed by the channel, strictly newer
/// than `current`, and carries both `Swoop-X.Y.Z.dmg` and `Swoop-X.Y.Z.dmg.sig`.
pub fn select_release(
    releases: &[GhRelease],
    include_prereleases: bool,
    current: &Version,
) -> Option<Candidate> {
    let mut best: Option<Candidate> = None;
    for r in releases {
        if r.draft {
            continue;
        }
        let Some(version) = parse_tag(&r.tag_name) else {
            tracing::debug!(tag = %r.tag_name, "skipping release with a non-semver tag");
            continue;
        };
        let is_pre = r.prerelease || !version.pre.is_empty();
        if is_pre && !include_prereleases {
            continue;
        }
        if !is_newer(&version, current) {
            continue;
        }
        if best
            .as_ref()
            .is_some_and(|b| !is_newer(&version, &b.version))
        {
            continue;
        }
        let find = |name: String| r.assets.iter().find(|a| a.name == name).cloned();
        let (Some(dmg), Some(sig)) = (
            find(dmg_asset_name(&version)),
            find(sig_asset_name(&version)),
        ) else {
            tracing::warn!(tag = %r.tag_name, "release is missing its DMG or signature asset");
            continue;
        };
        best = Some(Candidate {
            version,
            release: r.clone(),
            dmg,
            sig,
        });
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/releases.json");
    const LATEST: &str = include_str!("../tests/fixtures/release-latest.json");

    #[test]
    fn parses_the_releases_list() {
        let rs = parse_releases(FIXTURE.as_bytes()).unwrap();
        assert_eq!(rs.len(), 5);
        let r = &rs[1];
        assert_eq!(r.tag_name, "v0.2.0");
        assert_eq!(r.name.as_deref(), Some("Swoop 0.2.0"));
        assert!(!r.draft && !r.prerelease);
        assert_eq!(r.published_at.as_deref(), Some("2026-10-01T09:30:00Z"));
        assert_eq!(r.assets.len(), 2);
        assert_eq!(r.assets[0].name, "Swoop-0.2.0.dmg");
        assert_eq!(r.assets[0].size, 18_874_368);
        assert!(r.assets[0]
            .browser_download_url
            .starts_with("https://github.com/Saijayaranjan/Swoop/releases/download/v0.2.0/"));
        assert!(r.body.as_deref().unwrap().contains("## What's new"));
    }

    #[test]
    fn parses_latest_object() {
        let rs = parse_releases(LATEST.as_bytes()).unwrap();
        assert_eq!(rs.len(), 1);
        assert_eq!(rs[0].tag_name, "v0.1.1");
        let c = select_release(&rs, false, &Version::new(0, 1, 0)).unwrap();
        assert_eq!(c.version, Version::new(0, 1, 1));
        assert_eq!(c.dmg.name, "Swoop-0.1.1.dmg");
        assert_eq!(c.sig.name, "Swoop-0.1.1.dmg.sig");
    }

    #[test]
    fn selects_by_channel() {
        let rs = parse_releases(FIXTURE.as_bytes()).unwrap();
        let cur = Version::new(0, 1, 0);
        // stable: the draft (0.4.0), the prerelease (0.3.0-beta.1) and the asset-less 0.2.1 are skipped
        let stable = select_release(&rs, false, &cur).unwrap();
        assert_eq!(stable.version, Version::new(0, 2, 0));
        // beta opts into prereleases, but drafts are never offered
        let beta = select_release(&rs, true, &cur).unwrap();
        assert_eq!(beta.version.to_string(), "0.3.0-beta.1");
        // nothing newer than what's running
        assert!(select_release(&rs, false, &Version::new(0, 2, 0)).is_none());
        assert!(select_release(&rs, false, &Version::new(9, 0, 0)).is_none());
        assert!(select_release(&[], true, &cur).is_none());
    }
}
