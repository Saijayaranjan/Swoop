//! Automation runner: task events → matching automation rules → the `swoop-automation`
//! executor, with the consent gate, the run log and platform actions turned into events.

use crate::engine::Engine;
use parking_lot::RwLock;
use std::sync::Arc;
use swoop_automation::{consent_hash, ActionOutcome, Executor};
use swoop_domain::automation::{
    AutomationAction, AutomationContext, AutomationEvent, AutomationRule, AutomationRun,
    ConsentRecord,
};
use swoop_domain::schedule::Schedule;
use swoop_domain::{
    AutomationId, DomainError, DomainResult, Event, Millis, Notification, Task, TaskId,
};
use swoop_runtime::net::ClientFactory;

/// Owns the executor and the cached rule list.
pub struct AutomationRunner {
    executor: Executor,
    rules: RwLock<Vec<AutomationRule>>,
}

impl AutomationRunner {
    pub fn new(clients: Arc<ClientFactory>) -> Self {
        Self {
            executor: Executor::new(clients),
            rules: RwLock::new(Vec::new()),
        }
    }
    pub fn replace_all(&self, rules: Vec<AutomationRule>) {
        *self.rules.write() = rules;
    }
    pub fn all(&self) -> Vec<AutomationRule> {
        self.rules.read().clone()
    }
    pub fn get(&self, id: &AutomationId) -> Option<AutomationRule> {
        self.rules.read().iter().find(|r| &r.id == id).cloned()
    }
    pub fn upsert(&self, rule: AutomationRule) {
        let mut all = self.rules.write();
        match all.iter_mut().find(|r| r.id == rule.id) {
            Some(slot) => *slot = rule,
            None => all.push(rule),
        }
    }
    pub fn remove(&self, id: &AutomationId) {
        self.rules.write().retain(|r| &r.id != id);
    }

    /// Fire an event for a task: every enabled rule listening to it (and whose conditions
    /// match) runs, in creation order.
    pub async fn fire(&self, engine: &Engine, event: AutomationEvent, task: &Task) {
        let subject = crate::rules::subject_for(task);
        let mut rules: Vec<AutomationRule> = self
            .all()
            .into_iter()
            .filter(|r| r.enabled && r.events.contains(&event))
            .filter(|r| {
                r.conditions.is_empty()
                    || match r.match_mode {
                        swoop_domain::rules::MatchMode::All => {
                            r.conditions.iter().all(|c| c.matches(&subject))
                        }
                        swoop_domain::rules::MatchMode::Any => {
                            r.conditions.iter().any(|c| c.matches(&subject))
                        }
                    }
            })
            .collect();
        rules.sort_by_key(|r| r.created_at);
        for rule in rules {
            let ctx = engine.automation_context(task, event);
            let _ = self
                .run_rule(engine, &rule, ctx, event, Some(task.id.clone()))
                .await;
        }
    }

    /// `ScheduleFired` has no task; rules listening to it run with an empty context.
    pub async fn fire_schedule(&self, engine: &Engine, schedule: &Schedule, opened: bool) {
        let rules: Vec<AutomationRule> = self
            .all()
            .into_iter()
            .filter(|r| r.enabled && r.events.contains(&AutomationEvent::ScheduleFired))
            .collect();
        for rule in rules {
            let ctx = AutomationContext {
                event: format!(
                    "schedule_fired:{}:{}",
                    schedule.name,
                    if opened { "open" } else { "close" }
                ),
                date: Millis::now().to_datetime().format("%Y-%m-%d").to_string(),
                ..Default::default()
            };
            let _ = self
                .run_rule(engine, &rule, ctx, AutomationEvent::ScheduleFired, None)
                .await;
        }
    }

    /// Run one automation now (menu "Run automation", schedule actions, recipes).
    pub async fn run_by_id(
        &self,
        engine: &Engine,
        id: &AutomationId,
        task: Option<&Task>,
        event: AutomationEvent,
    ) -> DomainResult<AutomationRun> {
        let rule = self
            .get(id)
            .ok_or_else(|| DomainError::not_found(format!("automation {id}")))?;
        let ctx = match task {
            Some(t) => engine.automation_context(t, event),
            None => AutomationContext {
                event: format!("{event:?}").to_lowercase(),
                date: Millis::now().to_datetime().format("%Y-%m-%d").to_string(),
                ..Default::default()
            },
        };
        Ok(self
            .run_rule(engine, &rule, ctx, event, task.map(|t| t.id.clone()))
            .await)
    }

    async fn run_rule(
        &self,
        engine: &Engine,
        rule: &AutomationRule,
        mut ctx: AutomationContext,
        event: AutomationEvent,
        task_id: Option<TaskId>,
    ) -> AutomationRun {
        let consents = engine
            .store
            .consents_for(&rule.id)
            .await
            .unwrap_or_default();
        let mut messages: Vec<String> = Vec::new();
        let mut failure: Option<String> = None;
        for (index, action) in rule.actions.iter().enumerate() {
            let consent: Option<&ConsentRecord> =
                consents.iter().find(|c| c.action_index as usize == index);
            match self.executor.execute(action, &ctx, consent).await {
                Ok(outcome) => {
                    let msg = engine
                        .apply_action_outcome(rule, action, outcome, &mut ctx, task_id.as_ref())
                        .await;
                    messages.push(msg);
                }
                Err(e) => {
                    failure = Some(format!(
                        "action {index} ({}): {}",
                        action_name(action),
                        e.message
                    ));
                    break;
                }
            }
        }
        let success = failure.is_none();
        let run = AutomationRun {
            automation_id: rule.id.clone(),
            task_id,
            event,
            at: Millis::now(),
            success,
            message: swoop_runtime::redact::redact(&match &failure {
                Some(f) => f.clone(),
                None => messages.join("; "),
            }),
        };
        if let Err(e) = engine.store.append_automation_run(&run).await {
            tracing::debug!(error = %e, "automation run not recorded");
        }
        let mut updated = rule.clone();
        updated.run_count += 1;
        updated.last_run_at = Some(run.at);
        updated.last_error = failure.clone();
        self.upsert(updated.clone());
        if let Err(e) = engine.store.upsert_automation(&updated).await {
            tracing::debug!(error = %e, "automation counters not persisted");
        }
        engine.bus.publish(Event::AutomationUpdated(updated));
        engine.bus.publish(Event::AutomationRan(run.clone()));
        if let Some(err) = failure {
            engine.notify(Notification::AutomationFailed {
                automation_id: rule.id.clone(),
                name: rule.name.clone(),
                error: err,
            });
        }
        run
    }
}

fn action_name(a: &AutomationAction) -> &'static str {
    match a {
        AutomationAction::Move { .. } => "move",
        AutomationAction::Rename { .. } => "rename",
        AutomationAction::Copy { .. } => "copy",
        AutomationAction::Open => "open",
        AutomationAction::RevealInFinder => "reveal",
        AutomationAction::FinderTag { .. } => "finder_tag",
        AutomationAction::Notify { .. } => "notify",
        AutomationAction::Webhook { .. } => "webhook",
        AutomationAction::RunCommand { .. } => "run_command",
        AutomationAction::RunShell { .. } => "run_shell",
        AutomationAction::RunAppleScript { .. } => "run_applescript",
        AutomationAction::RunSandboxedScript { .. } => "run_script",
        AutomationAction::EmitEvent { .. } => "emit_event",
        AutomationAction::AddTag { .. } => "add_tag",
    }
}

impl Engine {
    /// Variables for automation actions.
    pub(crate) fn automation_context(
        &self,
        task: &Task,
        event: AutomationEvent,
    ) -> AutomationContext {
        let path = task.target_path();
        AutomationContext {
            task_id: task.id.clone(),
            event: format!("{event:?}").to_lowercase(),
            file_path: path.to_string_lossy().to_string(),
            file_name: path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| task.name.clone()),
            directory: path
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| task.directory.to_string_lossy().to_string()),
            extension: task.extension().unwrap_or_default(),
            source_url: task.source.primary_url().unwrap_or("").to_owned(),
            source_domain: task.domain().unwrap_or_default(),
            mime: task.mime.clone().unwrap_or_default(),
            queue: self
                .queues
                .get(&task.queue_id)
                .map(|q| q.name)
                .unwrap_or_default(),
            category: task
                .category_id
                .as_ref()
                .and_then(|c| {
                    self.categories
                        .read()
                        .iter()
                        .find(|x| &x.id == c)
                        .map(|x| x.name.clone())
                })
                .unwrap_or_default(),
            date: Millis::now().to_datetime().format("%Y-%m-%d").to_string(),
            size: task.progress.total.unwrap_or(task.progress.downloaded),
            checksum: task
                .verified_checksum
                .as_ref()
                .map(|c| c.value.clone())
                .unwrap_or_default(),
        }
    }

    /// Apply what an action produced: file moves update the task, platform actions become
    /// events, custom outcomes become events / tags. Returns a log message.
    async fn apply_action_outcome(
        &self,
        rule: &AutomationRule,
        action: &AutomationAction,
        outcome: ActionOutcome,
        ctx: &mut AutomationContext,
        task_id: Option<&TaskId>,
    ) -> String {
        match outcome {
            ActionOutcome::Done { message } => {
                if let AutomationAction::Notify { title, body } = action {
                    self.bus.publish(Event::Custom {
                        name: "automation.notify".into(),
                        payload: serde_json::json!({
                            "automation_id": rule.id,
                            "title": ctx.substitute(title),
                            "body": ctx.substitute(body),
                            "task_id": task_id,
                        }),
                    });
                }
                message
            }
            ActionOutcome::Renamed { new_path } | ActionOutcome::Moved { new_path } => {
                if let Some(id) = task_id {
                    self.relocate_task_file(id, &new_path).await;
                }
                ctx.file_path = new_path.to_string_lossy().to_string();
                ctx.file_name = new_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                ctx.directory = new_path
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                format!("{} -> {}", action_name(action), new_path.display())
            }
            ActionOutcome::Platform(a) => {
                self.bus.publish(Event::PlatformAction {
                    task_id: task_id.cloned(),
                    action: a,
                    context: ctx.clone(),
                });
                format!("{} handed to the platform layer", action_name(action))
            }
            ActionOutcome::Custom { name, payload } => {
                if let AutomationAction::AddTag { tags } = action {
                    if let Some(id) = task_id {
                        self.add_tags(id, tags).await;
                    }
                    format!("tags {tags:?}")
                } else {
                    self.bus.publish(Event::Custom {
                        name: name.clone(),
                        payload,
                    });
                    format!("event {name}")
                }
            }
        }
    }

    /// Record a moved/renamed completed file on the task.
    pub(crate) async fn relocate_task_file(&self, id: &TaskId, new_path: &std::path::Path) {
        let Some(cell) = self.tasks.get(id) else {
            return;
        };
        let snap = {
            let mut t = cell.lock();
            t.file_path = Some(new_path.to_path_buf());
            if let Some(n) = new_path.file_name() {
                t.name = n.to_string_lossy().to_string();
            }
            if let Some(p) = new_path.parent() {
                t.directory = p.to_path_buf();
            }
            t.touch();
            t.clone()
        };
        self.persist
            .send(crate::persist::PersistOp::Update(Box::new(snap.clone())));
        self.bus.publish(Event::TaskUpdated(Box::new(snap)));
    }

    async fn add_tags(&self, id: &TaskId, tags: &[String]) {
        let Some(cell) = self.tasks.get(id) else {
            return;
        };
        let snap = {
            let mut t = cell.lock();
            let mut changed = false;
            for tag in tags {
                if !t.tags.contains(tag) {
                    t.tags.push(tag.clone());
                    changed = true;
                }
            }
            if !changed {
                return;
            }
            t.touch();
            t.clone()
        };
        self.persist
            .send(crate::persist::PersistOp::Update(Box::new(snap.clone())));
        self.bus.publish(Event::TaskUpdated(Box::new(snap)));
    }

    fn validate_automation(a: &mut AutomationRule) -> DomainResult<()> {
        a.name = a.name.trim().to_owned();
        if a.name.is_empty() {
            return Err(DomainError::validation("automation name must not be empty"));
        }
        if a.actions.is_empty() {
            return Err(DomainError::validation(
                "automation needs at least one action",
            ));
        }
        for act in &a.actions {
            if let AutomationAction::Webhook { url, .. } = act {
                let u = url::Url::parse(url).map_err(DomainError::validation)?;
                let local = swoop_runtime::net::is_local_url(&u);
                if !(u.scheme() == "https" || (u.scheme() == "http" && local)) {
                    return Err(DomainError::validation(
                        "webhook URL must be https:// (or http://localhost)",
                    ));
                }
            }
            if let AutomationAction::RunCommand { program, .. } = act {
                if !std::path::Path::new(program).is_absolute() {
                    return Err(DomainError::validation(
                        "command program must be an absolute path",
                    ));
                }
            }
        }
        Ok(())
    }

    pub(crate) async fn create_automation_inner(
        &self,
        mut a: AutomationRule,
    ) -> DomainResult<AutomationRule> {
        Self::validate_automation(&mut a)?;
        if self.automation.get(&a.id).is_some() {
            return Err(DomainError::Conflict(format!("automation {} exists", a.id)));
        }
        let now = Millis::now();
        a.created_at = now;
        a.updated_at = now;
        a.run_count = 0;
        a.last_run_at = None;
        a.last_error = None;
        self.store.upsert_automation(&a).await?;
        self.automation.upsert(a.clone());
        self.bus.publish(Event::AutomationUpdated(a.clone()));
        Ok(a)
    }

    pub(crate) async fn update_automation_inner(
        &self,
        mut a: AutomationRule,
    ) -> DomainResult<AutomationRule> {
        Self::validate_automation(&mut a)?;
        let existing = self
            .automation
            .get(&a.id)
            .ok_or_else(|| DomainError::not_found(format!("automation {}", a.id)))?;
        a.created_at = existing.created_at;
        a.run_count = existing.run_count;
        a.last_run_at = existing.last_run_at;
        a.updated_at = Millis::now();
        self.store.upsert_automation(&a).await?;
        // Consent hashes are compared at execution time, so an edited command simply stops
        // matching; drop records whose action no longer needs consent to keep the table tidy.
        let consents = self.store.consents_for(&a.id).await.unwrap_or_default();
        let keep: Vec<ConsentRecord> = consents
            .into_iter()
            .filter(|c| {
                a.actions
                    .get(c.action_index as usize)
                    .and_then(consent_hash)
                    .map(|h| h == c.hash)
                    .unwrap_or(false)
            })
            .collect();
        self.store.grant_consents(&a.id, keep).await?;
        self.automation.upsert(a.clone());
        self.bus.publish(Event::AutomationUpdated(a.clone()));
        Ok(a)
    }

    pub(crate) async fn delete_automation_inner(&self, id: AutomationId) -> DomainResult<()> {
        if !self.store.delete_automation(&id).await? {
            return Err(DomainError::not_found(format!("automation {id}")));
        }
        self.automation.remove(&id);
        self.bus
            .publish(Event::AutomationRemoved { automation_id: id });
        Ok(())
    }

    pub(crate) async fn grant_consent_inner(
        &self,
        id: AutomationId,
    ) -> DomainResult<AutomationRule> {
        let rule = self
            .automation
            .get(&id)
            .ok_or_else(|| DomainError::not_found(format!("automation {id}")))?;
        let now = Millis::now();
        let records: Vec<ConsentRecord> = rule
            .actions
            .iter()
            .enumerate()
            .filter_map(|(i, a)| {
                consent_hash(a).map(|hash| ConsentRecord {
                    automation_id: id.clone(),
                    action_index: i as u32,
                    hash,
                    granted_at: now,
                })
            })
            .collect();
        self.store.grant_consents(&id, records).await?;
        Ok(rule)
    }

    pub(crate) async fn automation_runs_inner(
        &self,
        id: Option<AutomationId>,
        limit: u32,
    ) -> DomainResult<Vec<AutomationRun>> {
        Ok(self.store.automation_runs(id.as_ref(), limit).await?)
    }

    pub(crate) async fn run_automation_inner(
        &self,
        id: AutomationId,
        task_id: TaskId,
    ) -> DomainResult<AutomationRun> {
        let task = self
            .tasks
            .snapshot(&task_id)
            .ok_or_else(|| DomainError::not_found(format!("task {task_id}")))?;
        self.automation
            .run_by_id(self, &id, Some(&task), AutomationEvent::DownloadCompleted)
            .await
    }
}
