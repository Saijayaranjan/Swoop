//! Public types shared with the services API.

use osprey_domain::Millis;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GrabberOptions {
    pub url: String,
    /// 0 = only the start page.
    pub max_depth: u8,
    /// `same_domain`, `subdomains`, `external`.
    pub scope: String,
    pub respect_robots: bool,
    pub concurrency: u8,
    pub max_pages: u32,
    /// Lower-case extensions without dots; empty = the built-in "interesting" list.
    pub include_extensions: Vec<String>,
    /// Glob patterns (`*`, `?`) matched against the full URL; any match excludes.
    pub exclude_patterns: Vec<String>,
    pub include_regex: Option<String>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    /// Probe discovered files with `HEAD` for size/MIME.
    pub probe_files: bool,
    /// Also follow links inside `<iframe>`s.
    pub follow_iframes: bool,
}

impl Default for GrabberOptions {
    fn default() -> Self {
        Self {
            url: String::new(),
            max_depth: 1,
            scope: "same_domain".into(),
            respect_robots: true,
            concurrency: 4,
            max_pages: 200,
            include_extensions: Vec::new(),
            exclude_patterns: Vec::new(),
            include_regex: None,
            min_size: None,
            max_size: None,
            probe_files: true,
            follow_iframes: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GrabberFile {
    pub url: String,
    pub name: String,
    pub extension: String,
    pub domain: String,
    pub found_on: String,
    pub size: Option<u64>,
    pub mime: Option<String>,
    /// `document`, `image`, `video`, `audio`, `archive`, `software`, `torrent`, `playlist`, `other`.
    pub kind: String,
    pub depth: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GrabberSession {
    pub id: String,
    pub options: GrabberOptions,
    pub pages_crawled: u32,
    pub pages_queued: u32,
    pub files: Vec<GrabberFile>,
    pub done: bool,
    pub cancelled: bool,
    pub error: Option<String>,
    pub started_at: Millis,
    pub finished_at: Option<Millis>,
    /// Pages skipped because of robots.txt (for transparency).
    pub robots_blocked: u32,
}
