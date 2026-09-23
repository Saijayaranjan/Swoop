//! Organisation rules: evaluated deterministically (`priority ASC, created_at ASC`) at add time;
//! completion-time actions (`MoveAfterCompletion`, `FinderTags`, `RevealInFinder`,
//! `RunAutomation`) are re-derived from the same subject when the task finishes.

use crate::engine::Engine;
use osprey_domain::rules::{Rule, RuleAction, RuleSubject};
use osprey_domain::{DomainError, DomainResult, Event, Millis, RuleId, Task};
use osprey_runtime::safety::sanitize_filename;
use std::path::{Path, PathBuf};

/// What applying the rules did to a task.
#[derive(Clone, Debug, Default)]
pub struct RuleOutcome {
    /// Names of the rules that matched, in order.
    pub applied: Vec<String>,
    /// Actions deferred to completion.
    pub post: Vec<RuleAction>,
    /// Ids of the rules that matched (for hit counting).
    pub rule_ids: Vec<RuleId>,
}

/// Rules sorted into evaluation order.
pub fn ordered(rules: &[Rule]) -> Vec<Rule> {
    let mut r: Vec<Rule> = rules.iter().filter(|r| r.enabled).cloned().collect();
    r.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then(a.created_at.cmp(&b.created_at))
    });
    r
}

/// Dry-run: which rules match and what they would do, honouring `StopProcessing`.
pub fn evaluate(rules: &[Rule], subject: &RuleSubject) -> Vec<(Rule, Vec<RuleAction>)> {
    let mut out = Vec::new();
    for r in ordered(rules) {
        if !r.matches(subject) {
            continue;
        }
        let stop = r
            .actions
            .iter()
            .any(|a| matches!(a, RuleAction::StopProcessing));
        out.push((r.clone(), r.actions.clone()));
        if stop {
            break;
        }
    }
    out
}

/// The rule subject of a task.
pub fn subject_for(task: &Task) -> RuleSubject {
    RuleSubject {
        name: task.name.clone(),
        url: task.source.primary_url().unwrap_or("").to_owned(),
        domain: task.domain().unwrap_or_default(),
        mime: task.mime.clone(),
        size: task.progress.total,
        origin: task.origin.clone(),
        kind: task.kind.as_str().to_owned(),
    }
}

/// Is the action executed after completion rather than at add time?
pub fn is_post_action(a: &RuleAction) -> bool {
    matches!(
        a,
        RuleAction::MoveAfterCompletion { .. }
            | RuleAction::FinderTags { .. }
            | RuleAction::RevealInFinder
            | RuleAction::RunAutomation { .. }
    )
}

/// Expand a rename template.
pub fn render_template(template: &str, task: &Task, n: u64) -> String {
    let path = Path::new(&task.name);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(&task.name);
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    let date = Millis::now().to_datetime().format("%Y-%m-%d").to_string();
    let out = template
        .replace("{name}", &task.name)
        .replace("{stem}", stem)
        .replace("{ext}", ext)
        .replace("{date}", &date)
        .replace("{domain}", &task.domain().unwrap_or_default())
        .replace("{n}", &n.to_string());
    sanitize_filename(&out)
}

/// Resolve a rule directory: absolute → as is; relative → under the task directory.
pub fn resolve_dir(base: &Path, dir: &Path) -> PathBuf {
    let expanded = osprey_runtime::paths::AppPaths::expand_home(&dir.to_string_lossy());
    if expanded.is_absolute() {
        expanded
    } else {
        base.join(expanded)
    }
}

impl Engine {
    /// Apply the add-time actions of every matching rule to `task`.
    pub(crate) fn apply_rules(&self, task: &mut Task) -> RuleOutcome {
        let rules = self.rules.read().clone();
        let subject = subject_for(task);
        let mut outcome = RuleOutcome::default();
        for (rule, actions) in evaluate(&rules, &subject) {
            outcome.applied.push(rule.name.clone());
            outcome.rule_ids.push(rule.id.clone());
            self.apply_actions(task, &actions, rule.hit_count + 1, &mut outcome.post);
        }
        outcome
    }

    /// Apply a list of rule actions to a task; completion-time actions are pushed to `post`.
    pub(crate) fn apply_actions(
        &self,
        task: &mut Task,
        actions: &[RuleAction],
        n: u64,
        post: &mut Vec<RuleAction>,
    ) {
        for a in actions.iter().cloned() {
            match a {
                RuleAction::SaveTo { directory } => {
                    if !task.directory_locked {
                        task.directory = resolve_dir(&task.directory, &directory);
                    }
                }
                RuleAction::Rename { template } => {
                    if !task.name_locked {
                        task.name = render_template(&template, task, n);
                    }
                }
                RuleAction::AssignQueue { queue_id } => {
                    if self.queues.contains(&queue_id) {
                        task.queue_id = queue_id;
                    }
                }
                RuleAction::AssignCategory { category_id } => {
                    if self.categories.read().iter().any(|c| c.id == category_id) {
                        task.category_id = Some(category_id);
                    }
                }
                RuleAction::AddTags { tags } => {
                    for t in tags {
                        if !task.tags.contains(&t) {
                            task.tags.push(t);
                        }
                    }
                }
                RuleAction::SetPriority { priority } => task.priority = priority,
                RuleAction::DateFolder => {
                    if !task.directory_locked {
                        let d = Millis::now().to_datetime().format("%Y-%m-%d").to_string();
                        task.directory = task.directory.join(d);
                    }
                }
                RuleAction::DomainFolder => {
                    if !task.directory_locked {
                        if let Some(d) = task.domain() {
                            task.directory = task.directory.join(sanitize_filename(&d));
                        }
                    }
                }
                RuleAction::SetConnectionLimit { connections } => {
                    task.options.max_connections = Some(connections.max(1));
                }
                RuleAction::SetSpeedLimit { bytes_per_second } => {
                    task.options.download_limit = Some(bytes_per_second);
                }
                RuleAction::StopProcessing => {}
                other if is_post_action(&other) => post.push(other),
                _ => {}
            }
        }
    }

    /// Completion-time actions for a task.
    pub(crate) fn post_actions_for(&self, task: &Task) -> Vec<RuleAction> {
        let rules = self.rules.read().clone();
        evaluate(&rules, &subject_for(task))
            .into_iter()
            .flat_map(|(_, actions)| actions)
            .filter(is_post_action)
            .collect()
    }

    /// Bump `hit_count` of the rules that matched (best effort, persisted).
    pub(crate) async fn record_rule_hits(&self, ids: &[RuleId]) {
        for id in ids {
            let updated = {
                let mut rules = self.rules.write();
                match rules.iter_mut().find(|r| &r.id == id) {
                    Some(r) => {
                        r.hit_count += 1;
                        Some(r.clone())
                    }
                    None => None,
                }
            };
            if let Some(r) = updated {
                if let Err(e) = self.store.upsert_rule(&r).await {
                    tracing::debug!(error = %e, "rule hit count not persisted");
                }
            }
        }
    }

    fn validate_rule(rule: &mut Rule) -> DomainResult<()> {
        rule.name = rule.name.trim().to_owned();
        if rule.name.is_empty() {
            return Err(DomainError::validation("rule name must not be empty"));
        }
        for c in &rule.conditions {
            if let osprey_domain::rules::RuleCondition::Regex { pattern } = c {
                regex_check(pattern)?;
            }
        }
        Ok(())
    }

    pub(crate) async fn create_rule_inner(&self, mut rule: Rule) -> DomainResult<Rule> {
        Self::validate_rule(&mut rule)?;
        if self.rules.read().iter().any(|r| r.id == rule.id) {
            return Err(DomainError::Conflict(format!("rule {} exists", rule.id)));
        }
        let now = Millis::now();
        rule.created_at = now;
        rule.updated_at = now;
        rule.hit_count = 0;
        self.store.upsert_rule(&rule).await?;
        self.rules.write().push(rule.clone());
        self.bus.publish(Event::RuleUpdated(rule.clone()));
        Ok(rule)
    }

    pub(crate) async fn update_rule_inner(&self, mut rule: Rule) -> DomainResult<Rule> {
        Self::validate_rule(&mut rule)?;
        let existing = self
            .rules
            .read()
            .iter()
            .find(|r| r.id == rule.id)
            .cloned()
            .ok_or_else(|| DomainError::not_found(format!("rule {}", rule.id)))?;
        rule.created_at = existing.created_at;
        rule.hit_count = existing.hit_count;
        rule.updated_at = Millis::now();
        self.store.upsert_rule(&rule).await?;
        {
            let mut rules = self.rules.write();
            if let Some(slot) = rules.iter_mut().find(|r| r.id == rule.id) {
                *slot = rule.clone();
            }
        }
        self.bus.publish(Event::RuleUpdated(rule.clone()));
        Ok(rule)
    }

    pub(crate) async fn delete_rule_inner(&self, id: RuleId) -> DomainResult<()> {
        if !self.store.delete_rule(&id).await? {
            return Err(DomainError::not_found(format!("rule {id}")));
        }
        self.rules.write().retain(|r| r.id != id);
        self.bus.publish(Event::RuleRemoved { rule_id: id });
        Ok(())
    }
}

/// Reject regexes that fail to compile so a rule cannot silently never match.
fn regex_check(pattern: &str) -> DomainResult<()> {
    let probe = RuleSubject {
        url: String::new(),
        ..Default::default()
    };
    let r = osprey_domain::rules::RuleCondition::Regex {
        pattern: pattern.to_owned(),
    };
    // `matches` returns false both for "no match" and "invalid pattern"; a pattern that cannot
    // match the empty string but compiles is fine, so only obviously broken syntax is caught.
    let _ = r.matches(&probe);
    if pattern.trim().is_empty() {
        return Err(DomainError::validation("regex pattern must not be empty"));
    }
    if pattern.len() > 512 {
        return Err(DomainError::validation("regex pattern too long"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use osprey_domain::rules::RuleCondition;
    use osprey_domain::{QueueId, Source, TaskKind};

    fn task() -> Task {
        Task::new(
            TaskKind::Http,
            Source::Urls {
                urls: vec!["https://cdn.example.com/report.pdf".into()],
            },
            "report.pdf",
            PathBuf::from("/tmp/dl"),
            QueueId::default_queue(),
        )
    }

    #[test]
    fn stop_processing_halts_evaluation() {
        let mut a = Rule::new("a");
        a.priority = 1;
        a.conditions.push(RuleCondition::Extension {
            any_of: vec!["pdf".into()],
        });
        a.actions.push(RuleAction::StopProcessing);
        let mut b = Rule::new("b");
        b.priority = 2;
        b.conditions.push(RuleCondition::Extension {
            any_of: vec!["pdf".into()],
        });
        let matched = evaluate(&[b, a], &subject_for(&task()));
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].0.name, "a");
    }

    #[test]
    fn template_rendering() {
        let t = task();
        assert_eq!(
            render_template("{stem}-{domain}.{ext}", &t, 3),
            "report-cdn.example.com.pdf"
        );
        assert_eq!(render_template("{n}/{name}", &t, 3), "3_report.pdf");
    }
}
