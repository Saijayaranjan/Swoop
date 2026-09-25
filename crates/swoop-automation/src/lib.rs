//! Executes [`AutomationAction`]s with explicit permission boundaries.
//!
//! * Filesystem actions are confined to the task's directory tree / the user's home.
//! * Code-executing actions require a [`ConsentRecord`] whose hash matches the action.
//! * Commands run with an absolute program path, argv (never a shell) and variables in the
//!   environment; shell snippets receive variables only as environment variables.
//! * Webhooks are SSRF-checked. The sandboxed script runtime is a tiny expression language with
//!   no I/O (see [`sandbox`]).

pub mod sandbox;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use swoop_domain::automation::{AutomationAction, AutomationContext, ConsentRecord};
use swoop_domain::{ErrorKind, TaskError};
use swoop_runtime::net::ClientFactory;
use swoop_runtime::safety;

#[derive(Clone, Debug, PartialEq)]
pub enum ActionOutcome {
    Done {
        message: String,
    },
    Renamed {
        new_path: PathBuf,
    },
    Moved {
        new_path: PathBuf,
    },
    /// Must be executed by the platform layer (Finder, AppleScript, open).
    Platform(AutomationAction),
    Custom {
        name: String,
        payload: serde_json::Value,
    },
}

/// Hex BLAKE3 of the action's consent material, if it needs consent.
pub fn consent_hash(action: &AutomationAction) -> Option<String> {
    action
        .consent_material()
        .map(|m| blake3::hash(m.as_bytes()).to_hex().to_string())
}

pub fn requires_consent(action: &AutomationAction) -> bool {
    action.requires_consent()
}

pub struct Executor {
    clients: Arc<ClientFactory>,
}

impl Executor {
    pub fn new(clients: Arc<ClientFactory>) -> Self {
        Self { clients }
    }

    pub async fn execute(
        &self,
        action: &AutomationAction,
        ctx: &AutomationContext,
        consent: Option<&ConsentRecord>,
    ) -> Result<ActionOutcome, TaskError> {
        if action.requires_consent() {
            let expected = consent_hash(action).unwrap_or_default();
            match consent {
                Some(c) if constant_time_eq(c.hash.as_bytes(), expected.as_bytes()) => {}
                _ => {
                    return Err(TaskError::new(
                        ErrorKind::PermissionDenied,
                        "action has no matching user consent",
                    ))
                }
            }
        }
        match action {
            AutomationAction::Move { directory } => {
                let dst_dir = resolve_dir(directory, ctx)?;
                let src = PathBuf::from(&ctx.file_path);
                let dst = unique(dst_dir.join(file_name(&src)?));
                move_file(&src, &dst).await?;
                Ok(ActionOutcome::Moved { new_path: dst })
            }
            AutomationAction::Copy { directory } => {
                let dst_dir = resolve_dir(directory, ctx)?;
                let src = PathBuf::from(&ctx.file_path);
                let dst = unique(dst_dir.join(file_name(&src)?));
                tokio::fs::copy(&src, &dst)
                    .await
                    .map_err(|e| TaskError::from_io(&e, "copy"))?;
                Ok(ActionOutcome::Done {
                    message: format!("copied to {}", dst.display()),
                })
            }
            AutomationAction::Rename { template } => {
                let src = PathBuf::from(&ctx.file_path);
                let name = safety::sanitize_filename(&ctx.substitute(template));
                let parent = src
                    .parent()
                    .ok_or_else(|| TaskError::new(ErrorKind::InvalidFilename, "no parent"))?;
                let dst = unique(parent.join(name));
                tokio::fs::rename(&src, &dst)
                    .await
                    .map_err(|e| TaskError::from_io(&e, "rename"))?;
                Ok(ActionOutcome::Renamed { new_path: dst })
            }
            AutomationAction::Notify { title, body } => Ok(ActionOutcome::Done {
                message: format!("{}\n{}", ctx.substitute(title), ctx.substitute(body)),
            }),
            AutomationAction::Webhook { url, headers } => self.webhook(url, headers, ctx).await,
            AutomationAction::RunCommand { program, args } => run_command(program, args, ctx).await,
            AutomationAction::RunShell { script } => run_shell(script, ctx).await,
            AutomationAction::RunSandboxedScript { script, max_ms } => {
                let vars = ctx_vars(ctx);
                let result = sandbox::evaluate(
                    script,
                    &vars,
                    Duration::from_millis((*max_ms).clamp(10, 5_000) as u64),
                )?;
                Ok(ActionOutcome::Custom {
                    name: "script".into(),
                    payload: result,
                })
            }
            AutomationAction::EmitEvent { name, payload } => {
                let map: serde_json::Map<String, serde_json::Value> = payload
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::String(ctx.substitute(v))))
                    .collect();
                Ok(ActionOutcome::Custom {
                    name: name.clone(),
                    payload: serde_json::Value::Object(map),
                })
            }
            AutomationAction::AddTag { tags } => Ok(ActionOutcome::Custom {
                name: "add_tag".into(),
                payload: serde_json::json!({ "tags": tags }),
            }),
            AutomationAction::Open
            | AutomationAction::RevealInFinder
            | AutomationAction::FinderTag { .. }
            | AutomationAction::RunAppleScript { .. } => {
                Ok(ActionOutcome::Platform(action.clone()))
            }
        }
    }

    async fn webhook(
        &self,
        url: &str,
        headers: &std::collections::BTreeMap<String, String>,
        ctx: &AutomationContext,
    ) -> Result<ActionOutcome, TaskError> {
        let u = url::Url::parse(url)
            .map_err(|e| TaskError::new(ErrorKind::InvalidUrl, e.to_string()))?;
        if !matches!(u.scheme(), "https" | "http") {
            return Err(TaskError::new(
                ErrorKind::UnsupportedScheme,
                u.scheme().to_owned(),
            ));
        }
        // http:// only to localhost (local integrations); everything else must be https and public.
        if u.scheme() == "http" && !swoop_runtime::net::is_local_url(&u) {
            return Err(TaskError::new(
                ErrorKind::Forbidden,
                "plain http webhooks are only allowed to localhost",
            ));
        }
        if u.scheme() == "https" {
            swoop_runtime::net::assert_public_target(&u).await?;
        }
        let validated = swoop_runtime::net::validate_headers(
            headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
        )?;
        let client = self.clients.default_client()?;
        let mut req = client
            .post(u.clone())
            .timeout(Duration::from_secs(20))
            .json(&payload_for(ctx));
        for (k, v) in validated {
            req = req.header(k, ctx.substitute(&v));
        }
        let resp = req.send().await.map_err(|e| {
            TaskError::new(
                ErrorKind::ConnectionReset,
                swoop_runtime::redact::redact(&e.to_string()),
            )
        })?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(TaskError::from_http_status(status, url));
        }
        Ok(ActionOutcome::Done {
            message: format!("webhook {status}"),
        })
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn file_name(p: &Path) -> Result<std::ffi::OsString, TaskError> {
    p.file_name()
        .map(|n| n.to_owned())
        .ok_or_else(|| TaskError::new(ErrorKind::InvalidFilename, "no file name"))
}

fn unique(p: PathBuf) -> PathBuf {
    safety::unique_path(&p)
}

/// Resolve a target directory: templates substituted, `~` expanded, relative paths joined to the
/// task directory, then validated against the destination deny-list.
fn resolve_dir(dir: &Path, ctx: &AutomationContext) -> Result<PathBuf, TaskError> {
    let s = ctx.substitute(&dir.to_string_lossy());
    let p = swoop_runtime::paths::AppPaths::expand_home(&s);
    let p = if p.is_absolute() {
        p
    } else {
        PathBuf::from(&ctx.directory).join(p)
    };
    safety::validate_destination_dir(&p)?;
    Ok(p)
}

async fn move_file(src: &Path, dst: &Path) -> Result<(), TaskError> {
    if let Some(parent) = dst.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| TaskError::from_io(&e, "create dir"))?;
    }
    match tokio::fs::rename(src, dst).await {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(18) => {
            // EXDEV: cross-device — copy then remove
            tokio::fs::copy(src, dst)
                .await
                .map_err(|e| TaskError::from_io(&e, "copy"))?;
            tokio::fs::remove_file(src)
                .await
                .map_err(|e| TaskError::from_io(&e, "remove source"))?;
            Ok(())
        }
        Err(e) => Err(TaskError::from_io(&e, "move")),
    }
}

fn ctx_vars(ctx: &AutomationContext) -> std::collections::BTreeMap<String, serde_json::Value> {
    ctx.as_env()
        .into_iter()
        .map(|(k, v)| {
            (
                k.trim_start_matches("SWOOP_").to_ascii_lowercase(),
                serde_json::Value::String(v),
            )
        })
        .collect()
}

fn payload_for(ctx: &AutomationContext) -> serde_json::Value {
    serde_json::Value::Object(ctx_vars(ctx).into_iter().collect())
}

async fn run_command(
    program: &str,
    args: &[String],
    ctx: &AutomationContext,
) -> Result<ActionOutcome, TaskError> {
    let prog = Path::new(program);
    if !prog.is_absolute() {
        return Err(TaskError::new(
            ErrorKind::PermissionDenied,
            "program must be an absolute path",
        ));
    }
    let out = tokio::time::timeout(
        Duration::from_secs(300),
        tokio::process::Command::new(prog)
            .args(args.iter().map(|a| ctx.substitute(a)))
            .envs(ctx.as_env())
            .stdin(std::process::Stdio::null())
            .output(),
    )
    .await
    .map_err(|_| TaskError::new(ErrorKind::Internal, "command timed out after 300 s"))?
    .map_err(|e| TaskError::from_io(&e, "spawn"))?;
    finish_process(out)
}

async fn run_shell(script: &str, ctx: &AutomationContext) -> Result<ActionOutcome, TaskError> {
    let out = tokio::time::timeout(
        Duration::from_secs(300),
        tokio::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .envs(ctx.as_env())
            .stdin(std::process::Stdio::null())
            .output(),
    )
    .await
    .map_err(|_| TaskError::new(ErrorKind::Internal, "script timed out after 300 s"))?
    .map_err(|e| TaskError::from_io(&e, "spawn sh"))?;
    finish_process(out)
}

fn finish_process(out: std::process::Output) -> Result<ActionOutcome, TaskError> {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let tail = |s: &str| {
        s.chars()
            .rev()
            .take(2000)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<String>()
    };
    if out.status.success() {
        Ok(ActionOutcome::Done {
            message: swoop_runtime::redact::redact(&tail(&stdout)),
        })
    } else {
        Err(TaskError::new(
            ErrorKind::Internal,
            format!(
                "exit {}: {}",
                out.status.code().unwrap_or(-1),
                swoop_runtime::redact::redact(&tail(&stderr))
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swoop_domain::settings::Settings;
    use swoop_domain::{AutomationId, Millis, TaskId};

    fn ctx(dir: &Path, file: &Path) -> AutomationContext {
        AutomationContext {
            task_id: TaskId::new(),
            event: "download_completed".into(),
            file_path: file.to_string_lossy().into(),
            file_name: file.file_name().unwrap().to_string_lossy().into(),
            directory: dir.to_string_lossy().into(),
            extension: "txt".into(),
            source_url: "https://example.com/a.txt".into(),
            source_domain: "example.com".into(),
            mime: "text/plain".into(),
            queue: "Default".into(),
            category: "Documents".into(),
            date: "2026-09-19".into(),
            size: 5,
            checksum: String::new(),
        }
    }

    fn exec() -> Executor {
        Executor::new(ClientFactory::new(Arc::new(Settings::default())))
    }

    #[tokio::test]
    async fn move_rename_copy() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.txt");
        std::fs::write(&f, b"hello").unwrap();
        let c = ctx(dir.path(), &f);
        let r = exec()
            .execute(
                &AutomationAction::Rename {
                    template: "{date}-{name}".into(),
                },
                &c,
                None,
            )
            .await
            .unwrap();
        let renamed = match r {
            ActionOutcome::Renamed { new_path } => new_path,
            o => panic!("{o:?}"),
        };
        assert!(renamed.ends_with("2026-09-19-a.txt"));
        let c2 = ctx(dir.path(), &renamed);
        let r = exec()
            .execute(
                &AutomationAction::Move {
                    directory: PathBuf::from("{domain}"),
                },
                &c2,
                None,
            )
            .await
            .unwrap();
        match r {
            ActionOutcome::Moved { new_path } => {
                assert!(new_path.starts_with(dir.path().join("example.com")))
            }
            o => panic!("{o:?}"),
        }
        // rename with traversal in template is sanitised
        let f3 = dir.path().join("b.txt");
        std::fs::write(&f3, b"x").unwrap();
        let r = exec()
            .execute(
                &AutomationAction::Rename {
                    template: "../../{name}".into(),
                },
                &ctx(dir.path(), &f3),
                None,
            )
            .await
            .unwrap();
        match r {
            ActionOutcome::Renamed { new_path } => {
                assert_eq!(new_path.parent().unwrap(), dir.path())
            }
            o => panic!("{o:?}"),
        }
    }

    #[tokio::test]
    async fn consent_gate() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.txt");
        std::fs::write(&f, b"hello").unwrap();
        let c = ctx(dir.path(), &f);
        let action = AutomationAction::RunCommand {
            program: "/bin/echo".into(),
            args: vec!["{name}".into()],
        };
        assert!(exec().execute(&action, &c, None).await.is_err());
        let bad = ConsentRecord {
            automation_id: AutomationId::new(),
            action_index: 0,
            hash: "00".into(),
            granted_at: Millis::now(),
        };
        assert!(exec().execute(&action, &c, Some(&bad)).await.is_err());
        let good = ConsentRecord {
            automation_id: AutomationId::new(),
            action_index: 0,
            hash: consent_hash(&action).unwrap(),
            granted_at: Millis::now(),
        };
        match exec().execute(&action, &c, Some(&good)).await.unwrap() {
            ActionOutcome::Done { message } => assert_eq!(message.trim(), "a.txt"),
            o => panic!("{o:?}"),
        }
        // editing the command invalidates consent
        let edited = AutomationAction::RunCommand {
            program: "/bin/echo".into(),
            args: vec!["pwned".into()],
        };
        assert!(exec().execute(&edited, &c, Some(&good)).await.is_err());
        // relative program refused even with consent
        let rel = AutomationAction::RunCommand {
            program: "echo".into(),
            args: vec![],
        };
        let rc = ConsentRecord {
            hash: consent_hash(&rel).unwrap(),
            ..good.clone()
        };
        assert!(exec().execute(&rel, &c, Some(&rc)).await.is_err());
        // shell gets variables via env, not interpolation
        let sh = AutomationAction::RunShell {
            script: "printf '%s' \"$SWOOP_FILE_NAME\"".into(),
        };
        let sc = ConsentRecord {
            hash: consent_hash(&sh).unwrap(),
            ..good.clone()
        };
        match exec().execute(&sh, &c, Some(&sc)).await.unwrap() {
            ActionOutcome::Done { message } => assert_eq!(message, "a.txt"),
            o => panic!("{o:?}"),
        }
    }

    #[tokio::test]
    async fn webhook_and_platform() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.txt");
        let c = ctx(dir.path(), &f);
        // http to a public host is refused; https to a private host is refused
        assert!(exec()
            .execute(
                &AutomationAction::Webhook {
                    url: "http://example.com/x".into(),
                    headers: Default::default()
                },
                &c,
                None
            )
            .await
            .is_err());
        assert!(exec()
            .execute(
                &AutomationAction::Webhook {
                    url: "https://127.0.0.1/x".into(),
                    headers: Default::default()
                },
                &c,
                None
            )
            .await
            .is_err());
        // localhost http works
        let server = swoop_testserver::TestServer::start().await;
        server.add_text("hook", "application/json", "{}");
        // test server only accepts GET on /text; a POST returns 405 → error surfaces as HTTP status
        let r = exec()
            .execute(
                &AutomationAction::Webhook {
                    url: server.url("/text/hook"),
                    headers: Default::default(),
                },
                &c,
                None,
            )
            .await;
        assert!(matches!(r, Err(e) if e.status_code == Some(405)));
        assert_eq!(
            exec()
                .execute(&AutomationAction::RevealInFinder, &c, None)
                .await
                .unwrap(),
            ActionOutcome::Platform(AutomationAction::RevealInFinder)
        );
        match exec()
            .execute(
                &AutomationAction::Notify {
                    title: "Done {name}".into(),
                    body: "b".into(),
                },
                &c,
                None,
            )
            .await
            .unwrap()
        {
            ActionOutcome::Done { message } => assert!(message.starts_with("Done a.txt")),
            o => panic!("{o:?}"),
        }
    }
}
