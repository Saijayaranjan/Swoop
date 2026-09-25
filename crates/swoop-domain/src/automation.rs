//! Post-download automation: events → actions with explicit permission boundaries.

use crate::{AutomationId, Millis, TaskId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationEvent {
    DownloadStarted,
    DownloadPaused,
    DownloadResumed,
    DownloadFailed,
    DownloadCompleted,
    DownloadVerified,
    TorrentStarted,
    TorrentFinished,
    QueueFinished,
    ScheduleFired,
}

impl AutomationEvent {
    pub const ALL: [AutomationEvent; 10] = [
        Self::DownloadStarted,
        Self::DownloadPaused,
        Self::DownloadResumed,
        Self::DownloadFailed,
        Self::DownloadCompleted,
        Self::DownloadVerified,
        Self::TorrentStarted,
        Self::TorrentFinished,
        Self::QueueFinished,
        Self::ScheduleFired,
    ];
}

/// Actions. Anything that executes external code requires a [`ConsentRecord`] stored by the
/// services layer (never inside the rule JSON, which a remote admin could author). The record
/// holds a BLAKE3 hash of the exact command/script the user approved through the local UI; the
/// engine refuses to run an action whose current definition hashes differently, so editing a
/// command silently revokes consent. Remote clients cannot create or modify these actions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum AutomationAction {
    Move {
        directory: PathBuf,
    },
    Rename {
        template: String,
    },
    Copy {
        directory: PathBuf,
    },
    Open,
    RevealInFinder,
    FinderTag {
        tags: Vec<String>,
    },
    Notify {
        title: String,
        body: String,
    },
    /// POST JSON to a URL. Only https:// or http://localhost are accepted.
    Webhook {
        url: String,
        headers: BTreeMap<String, String>,
    },
    /// Run an executable with arguments. `program` must be an absolute path (no PATH lookup);
    /// variables are substituted per argument, never through a shell.
    RunCommand {
        program: String,
        args: Vec<String>,
    },
    /// Run a shell snippet via `/bin/sh -c`. Variables are exported as environment variables
    /// (`SWOOP_FILE_PATH`, …) and never interpolated into the script text.
    RunShell {
        script: String,
    },
    /// macOS only: executed by the app layer through `NSAppleScript`.
    RunAppleScript {
        script: String,
    },
    /// Runs inside the sandboxed expression mini-runtime (no I/O, no network; receives
    /// variables, returns an optional new filename/directory). Limited to `max_ms` CPU time.
    RunSandboxedScript {
        script: String,
        max_ms: u32,
    },
    /// Emit a local event on the WebSocket stream for external integrations.
    EmitEvent {
        name: String,
        payload: BTreeMap<String, String>,
    },
    /// Add the file to another queue as a new task (e.g. mirror to a NAS via FTP upload later).
    AddTag {
        tags: Vec<String>,
    },
}

impl AutomationAction {
    /// Actions that execute code outside the core and therefore need consent.
    pub fn requires_consent(&self) -> bool {
        matches!(
            self,
            AutomationAction::RunCommand { .. }
                | AutomationAction::RunShell { .. }
                | AutomationAction::RunAppleScript { .. }
                | AutomationAction::RunSandboxedScript { .. }
        )
    }

    /// Canonical string whose BLAKE3 hash is recorded as consent.
    pub fn consent_material(&self) -> Option<String> {
        match self {
            AutomationAction::RunCommand { program, args } => {
                let mut s = format!("cmd\0{program}");
                for a in args {
                    s.push('\0');
                    s.push_str(a);
                }
                Some(s)
            }
            AutomationAction::RunShell { script } => Some(format!("sh\0{script}")),
            AutomationAction::RunAppleScript { script } => Some(format!("applescript\0{script}")),
            AutomationAction::RunSandboxedScript { script, .. } => Some(format!("js\0{script}")),
            _ => None,
        }
    }

    /// True if this action must be executed by the platform layer (Swift/WinUI), not the core.
    pub fn is_platform_action(&self) -> bool {
        matches!(
            self,
            AutomationAction::Open
                | AutomationAction::RevealInFinder
                | AutomationAction::FinderTag { .. }
                | AutomationAction::RunAppleScript { .. }
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationRule {
    pub id: AutomationId,
    pub name: String,
    pub enabled: bool,
    pub events: Vec<AutomationEvent>,
    /// Optional filter (reuses organisation-rule conditions).
    #[serde(default)]
    pub conditions: Vec<crate::rules::RuleCondition>,
    #[serde(default)]
    pub match_mode: crate::rules::MatchMode,
    pub actions: Vec<AutomationAction>,
    #[serde(default)]
    pub run_count: u64,
    #[serde(default)]
    pub last_run_at: Option<Millis>,
    #[serde(default)]
    pub last_error: Option<String>,
    pub created_at: Millis,
    pub updated_at: Millis,
}

impl AutomationRule {
    pub fn new(name: impl Into<String>) -> Self {
        let now = Millis::now();
        Self {
            id: AutomationId::new(),
            name: name.into(),
            enabled: true,
            events: Vec::new(),
            conditions: Vec::new(),
            match_mode: crate::rules::MatchMode::All,
            actions: Vec::new(),
            run_count: 0,
            last_run_at: None,
            last_error: None,
            created_at: now,
            updated_at: now,
        }
    }
}

/// Variables available to automation actions.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AutomationContext {
    pub task_id: TaskId,
    pub event: String,
    pub file_path: String,
    pub file_name: String,
    pub directory: String,
    pub extension: String,
    pub source_url: String,
    pub source_domain: String,
    pub mime: String,
    pub queue: String,
    pub category: String,
    pub date: String,
    pub size: u64,
    pub checksum: String,
}

impl AutomationContext {
    /// Environment-variable form (`SWOOP_FILE_PATH`, …) for commands and scripts.
    pub fn as_env(&self) -> Vec<(String, String)> {
        vec![
            ("SWOOP_TASK_ID".into(), self.task_id.0.clone()),
            ("SWOOP_EVENT".into(), self.event.clone()),
            ("SWOOP_FILE_PATH".into(), self.file_path.clone()),
            ("SWOOP_FILE_NAME".into(), self.file_name.clone()),
            ("SWOOP_DIRECTORY".into(), self.directory.clone()),
            ("SWOOP_EXTENSION".into(), self.extension.clone()),
            ("SWOOP_SOURCE_URL".into(), self.source_url.clone()),
            ("SWOOP_SOURCE_DOMAIN".into(), self.source_domain.clone()),
            ("SWOOP_MIME".into(), self.mime.clone()),
            ("SWOOP_QUEUE".into(), self.queue.clone()),
            ("SWOOP_CATEGORY".into(), self.category.clone()),
            ("SWOOP_DATE".into(), self.date.clone()),
            ("SWOOP_SIZE".into(), self.size.to_string()),
            ("SWOOP_CHECKSUM".into(), self.checksum.clone()),
        ]
    }

    /// Substitute `{file_path}`-style placeholders in a template.
    pub fn substitute(&self, template: &str) -> String {
        let mut out = template.to_owned();
        for (k, v) in [
            ("{task_id}", self.task_id.0.as_str()),
            ("{event}", self.event.as_str()),
            ("{file_path}", self.file_path.as_str()),
            ("{file_name}", self.file_name.as_str()),
            ("{name}", self.file_name.as_str()),
            ("{directory}", self.directory.as_str()),
            ("{extension}", self.extension.as_str()),
            ("{ext}", self.extension.as_str()),
            ("{source_url}", self.source_url.as_str()),
            ("{url}", self.source_url.as_str()),
            ("{source_domain}", self.source_domain.as_str()),
            ("{domain}", self.source_domain.as_str()),
            ("{mime}", self.mime.as_str()),
            ("{queue}", self.queue.as_str()),
            ("{category}", self.category.as_str()),
            ("{date}", self.date.as_str()),
            ("{checksum}", self.checksum.as_str()),
        ] {
            out = out.replace(k, v);
        }
        out = out.replace("{size}", &self.size.to_string());
        let stem = std::path::Path::new(&self.file_name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        out.replace("{stem}", stem)
    }
}

/// A user's approval of one code-executing action. Stored by the services layer and written
/// only through the local (non-remote) API.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentRecord {
    pub automation_id: AutomationId,
    pub action_index: u32,
    /// Hex BLAKE3 of [`AutomationAction::consent_material`].
    pub hash: String,
    pub granted_at: Millis,
}

/// Record of an automation run, kept for the Automation log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationRun {
    pub automation_id: AutomationId,
    pub task_id: Option<TaskId>,
    pub event: AutomationEvent,
    pub at: Millis,
    pub success: bool,
    pub message: String,
}
