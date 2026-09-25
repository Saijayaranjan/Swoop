//! Organisation rules: deterministic, ordered, visible.

use crate::{CategoryId, Millis, QueueId, RuleId};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "field")]
pub enum RuleCondition {
    Extension {
        any_of: Vec<String>,
    },
    Mime {
        prefix: String,
    },
    /// Glob on the file name (`*.iso`, `report-*.pdf`).
    Filename {
        glob: String,
    },
    /// Substring or glob on the source URL.
    Url {
        contains: String,
    },
    Domain {
        any_of: Vec<String>,
    },
    /// Regex on the full URL.
    Regex {
        pattern: String,
    },
    /// Size in bytes.
    SizeGreaterThan {
        bytes: u64,
    },
    SizeLessThan {
        bytes: u64,
    },
    /// Where the task came from (`browser`, `cli`, `grabber`, `remote`, `recipe:*`).
    Origin {
        equals: String,
    },
    Kind {
        equals: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "action")]
pub enum RuleAction {
    SaveTo {
        directory: PathBuf,
    },
    /// Template with `{name}`, `{stem}`, `{ext}`, `{date}`, `{domain}`, `{n}` placeholders.
    Rename {
        template: String,
    },
    AssignQueue {
        queue_id: QueueId,
    },
    AssignCategory {
        category_id: CategoryId,
    },
    AddTags {
        tags: Vec<String>,
    },
    SetPriority {
        priority: crate::Priority,
    },
    /// Create a `YYYY-MM-DD` subfolder.
    DateFolder,
    /// Create a `<domain>` subfolder.
    DomainFolder,
    /// Move after completion (relative to destination or absolute).
    MoveAfterCompletion {
        directory: PathBuf,
    },
    /// macOS Finder tag names applied by the app layer.
    FinderTags {
        tags: Vec<String>,
    },
    RevealInFinder,
    /// Run an automation rule by id after completion.
    RunAutomation {
        automation_id: crate::AutomationId,
    },
    SetConnectionLimit {
        connections: u8,
    },
    SetSpeedLimit {
        bytes_per_second: u64,
    },
    /// Stop evaluating lower-priority rules after this one matched.
    StopProcessing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MatchMode {
    #[default]
    All,
    Any,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub id: RuleId,
    pub name: String,
    pub enabled: bool,
    /// Lower runs first. Ties are broken by `created_at` so the order is deterministic.
    pub priority: i32,
    pub match_mode: MatchMode,
    pub conditions: Vec<RuleCondition>,
    pub actions: Vec<RuleAction>,
    pub created_at: Millis,
    pub updated_at: Millis,
    /// How many tasks this rule has affected (for the UI).
    #[serde(default)]
    pub hit_count: u64,
}

impl Rule {
    pub fn new(name: impl Into<String>) -> Self {
        let now = Millis::now();
        Self {
            id: RuleId::new(),
            name: name.into(),
            enabled: true,
            priority: 100,
            match_mode: MatchMode::All,
            conditions: Vec::new(),
            actions: Vec::new(),
            created_at: now,
            updated_at: now,
            hit_count: 0,
        }
    }
}

/// The subset of task data rules are evaluated against (kept small on purpose so the engine can
/// evaluate rules before the task exists).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RuleSubject {
    pub name: String,
    pub url: String,
    pub domain: String,
    pub mime: Option<String>,
    pub size: Option<u64>,
    pub origin: String,
    pub kind: String,
}

impl RuleSubject {
    pub fn extension(&self) -> String {
        std::path::Path::new(&self.name)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default()
    }
}

/// Glob matcher supporting `*` and `?` (case-insensitive). Iterative two-pointer backtracking:
/// O(n·m) worst case, so untrusted patterns from the remote API cannot blow up the CPU.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().map(|c| c.to_ascii_lowercase()).collect();
    let t: Vec<char> = text.chars().map(|c| c.to_ascii_lowercase()).collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star_p, mut star_t): (Option<usize>, usize) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star_p = Some(pi);
            star_t = ti;
            pi += 1;
        } else if let Some(sp) = star_p {
            pi = sp + 1;
            star_t += 1;
            ti = star_t;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Compiled-regex cache so rule evaluation does not recompile on every task. Bounded so a
/// stream of unique patterns cannot grow it without limit.
fn cached_regex(pattern: &str) -> Option<regex::Regex> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, Option<regex::Regex>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(r) = guard.get(pattern) {
        return r.clone();
    }
    if guard.len() >= 256 {
        guard.clear();
    }
    let compiled = regex::RegexBuilder::new(pattern)
        .size_limit(1 << 20)
        .dfa_size_limit(1 << 20)
        .build()
        .ok();
    guard.insert(pattern.to_owned(), compiled.clone());
    compiled
}

impl RuleCondition {
    pub fn matches(&self, s: &RuleSubject) -> bool {
        match self {
            RuleCondition::Extension { any_of } => {
                let ext = s.extension();
                any_of
                    .iter()
                    .any(|e| e.trim_start_matches('.').eq_ignore_ascii_case(&ext))
            }
            RuleCondition::Mime { prefix } => s
                .mime
                .as_deref()
                .map(|m| {
                    m.to_ascii_lowercase()
                        .starts_with(&prefix.to_ascii_lowercase())
                })
                .unwrap_or(false),
            RuleCondition::Filename { glob } => glob_match(glob, &s.name),
            RuleCondition::Url { contains } => {
                if contains.contains('*') || contains.contains('?') {
                    glob_match(contains, &s.url)
                } else {
                    s.url
                        .to_ascii_lowercase()
                        .contains(&contains.to_ascii_lowercase())
                }
            }
            RuleCondition::Domain { any_of } => any_of.iter().any(|d| {
                let d = d.to_ascii_lowercase();
                let host = s.domain.to_ascii_lowercase();
                host == d || host.ends_with(&format!(".{d}")) || glob_match(&d, &host)
            }),
            RuleCondition::Regex { pattern } => cached_regex(pattern)
                .map(|r| r.is_match(&s.url))
                .unwrap_or(false),
            RuleCondition::SizeGreaterThan { bytes } => s.size.map(|z| z > *bytes).unwrap_or(false),
            RuleCondition::SizeLessThan { bytes } => s.size.map(|z| z < *bytes).unwrap_or(false),
            RuleCondition::Origin { equals } => {
                s.origin.eq_ignore_ascii_case(equals) || s.origin.starts_with(&format!("{equals}:"))
            }
            RuleCondition::Kind { equals } => s.kind.eq_ignore_ascii_case(equals),
        }
    }
}

impl Rule {
    pub fn matches(&self, s: &RuleSubject) -> bool {
        if !self.enabled || self.conditions.is_empty() {
            return false;
        }
        match self.match_mode {
            MatchMode::All => self.conditions.iter().all(|c| c.matches(s)),
            MatchMode::Any => self.conditions.iter().any(|c| c.matches(s)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob() {
        assert!(glob_match("*.iso", "ubuntu.ISO"));
        assert!(glob_match("report-??.pdf", "report-01.pdf"));
        assert!(!glob_match("report-??.pdf", "report-001.pdf"));
        assert!(glob_match("**", ""));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("a*b*c", "aXXbYYc"));
        assert!(!glob_match("a*b*c", "aXXbYY"));
        // pathological pattern must return quickly
        let start = std::time::Instant::now();
        assert!(!glob_match(
            &"*a".repeat(30),
            &"a".repeat(60).replace("aaa", "aab")
        ));
        assert!(start.elapsed().as_millis() < 500);
    }

    #[test]
    fn rule_matching() {
        let mut r = Rule::new("pdfs");
        r.conditions.push(RuleCondition::Extension {
            any_of: vec!["pdf".into()],
        });
        r.conditions.push(RuleCondition::Domain {
            any_of: vec!["example.com".into()],
        });
        let s = RuleSubject {
            name: "x.PDF".into(),
            url: "https://cdn.example.com/x.pdf".into(),
            domain: "cdn.example.com".into(),
            ..Default::default()
        };
        assert!(r.matches(&s));
        r.match_mode = MatchMode::All;
        r.conditions
            .push(RuleCondition::SizeGreaterThan { bytes: 10 });
        assert!(!r.matches(&s));
        r.match_mode = MatchMode::Any;
        assert!(r.matches(&s));
    }
}
