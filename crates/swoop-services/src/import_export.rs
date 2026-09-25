//! JSON export / import of configuration (and optionally tasks + history).

use crate::api::{ExportBundle, ImportOptions, ImportReport, Recipe};
use crate::engine::Engine;
use swoop_domain::{DomainResult, Event, Millis, TaskState};

/// Bundle format version.
pub const EXPORT_SCHEMA_VERSION: u32 = 1;

impl Engine {
    pub(crate) async fn export_inner(
        &self,
        include_tasks: bool,
        include_history: bool,
    ) -> DomainResult<ExportBundle> {
        let history = if include_history {
            self.store
                .query_history(&swoop_domain::history::HistoryQuery {
                    limit: 0,
                    ..Default::default()
                })
                .await?
        } else {
            Vec::new()
        };
        let tasks = if include_tasks {
            self.tasks.all()
        } else {
            Vec::new()
        };
        let recipes = self.list_recipes_inner().await?;
        Ok(ExportBundle {
            schema_version: EXPORT_SCHEMA_VERSION,
            exported_at: Millis::now(),
            settings: Some((*self.settings()).clone()),
            queues: self.queues.all(),
            categories: self.categories.read().clone(),
            rules: self.rules.read().clone(),
            schedules: self.scheduler.all(),
            automations: self.automation.all(),
            tasks,
            history,
            recipes,
        })
    }

    pub(crate) async fn import_inner(
        &self,
        bundle: ExportBundle,
        options: ImportOptions,
    ) -> DomainResult<ImportReport> {
        let mut report = ImportReport::default();
        let mut bump = |section: &str, ok: bool| {
            let map = if ok {
                &mut report.imported
            } else {
                &mut report.skipped
            };
            *map.entry(section.to_owned()).or_insert(0) += 1;
        };

        if options.settings {
            if let Some(s) = bundle.settings {
                match self.update_settings_inner(s).await {
                    Ok(_) => bump("settings", true),
                    Err(e) => {
                        bump("settings", false);
                        report.errors.push(format!("settings: {e}"));
                    }
                }
            }
        }
        if options.queues {
            for q in bundle.queues {
                let exists = self.queues.contains(&q.id);
                let r = if exists && options.overwrite {
                    self.update_queue_inner(q).await
                } else if !exists {
                    self.create_queue_inner(q).await
                } else {
                    bump("queues", false);
                    continue;
                };
                match r {
                    Ok(_) => bump("queues", true),
                    Err(e) => {
                        bump("queues", false);
                        report.errors.push(format!("queue: {e}"));
                    }
                }
            }
        }
        if options.categories {
            for c in bundle.categories {
                let exists = self.categories.read().iter().any(|x| x.id == c.id);
                let r = if exists && options.overwrite {
                    self.update_category_inner(c).await
                } else if !exists {
                    self.create_category_inner(c).await
                } else {
                    bump("categories", false);
                    continue;
                };
                match r {
                    Ok(_) => bump("categories", true),
                    Err(e) => {
                        bump("categories", false);
                        report.errors.push(format!("category: {e}"));
                    }
                }
            }
        }
        if options.rules {
            for r in bundle.rules {
                let exists = self.rules.read().iter().any(|x| x.id == r.id);
                let res = if exists && options.overwrite {
                    self.update_rule_inner(r).await
                } else if !exists {
                    self.create_rule_inner(r).await
                } else {
                    bump("rules", false);
                    continue;
                };
                match res {
                    Ok(_) => bump("rules", true),
                    Err(e) => {
                        bump("rules", false);
                        report.errors.push(format!("rule: {e}"));
                    }
                }
            }
        }
        if options.schedules {
            for s in bundle.schedules {
                let exists = self.scheduler.contains(&s.id);
                let res = if exists && options.overwrite {
                    self.update_schedule_inner(s).await
                } else if !exists {
                    self.create_schedule_inner(s).await
                } else {
                    bump("schedules", false);
                    continue;
                };
                match res {
                    Ok(_) => bump("schedules", true),
                    Err(e) => {
                        bump("schedules", false);
                        report.errors.push(format!("schedule: {e}"));
                    }
                }
            }
        }
        if options.automations {
            for a in bundle.automations {
                let exists = self.automation.get(&a.id).is_some();
                let res = if exists && options.overwrite {
                    self.update_automation_inner(a).await
                } else if !exists {
                    self.create_automation_inner(a).await
                } else {
                    bump("automations", false);
                    continue;
                };
                // Imported automations never carry consent; code-executing actions stay
                // disabled until the user grants it locally.
                match res {
                    Ok(_) => bump("automations", true),
                    Err(e) => {
                        bump("automations", false);
                        report.errors.push(format!("automation: {e}"));
                    }
                }
            }
        }
        if options.recipes {
            for r in bundle.recipes {
                let existing: Vec<Recipe> = self.list_recipes_inner().await.unwrap_or_default();
                let exists = existing.iter().any(|x| x.id == r.id);
                if exists && !options.overwrite {
                    bump("recipes", false);
                    continue;
                }
                match self.save_recipe_inner(r).await {
                    Ok(_) => bump("recipes", true),
                    Err(e) => {
                        bump("recipes", false);
                        report.errors.push(format!("recipe: {e}"));
                    }
                }
            }
        }
        if options.history {
            for h in bundle.history {
                match self.store.insert_history(&h).await {
                    Ok(()) => bump("history", true),
                    Err(e) => {
                        bump("history", false);
                        report.errors.push(format!("history: {e}"));
                    }
                }
            }
        }
        if options.tasks {
            for mut t in bundle.tasks {
                if self.tasks.contains(&t.id) && !options.overwrite {
                    bump("tasks", false);
                    continue;
                }
                if self.tasks.run(&t.id).is_some() {
                    bump("tasks", false);
                    continue;
                }
                // Imported tasks never resume a transfer that was live elsewhere.
                if t.state.is_active()
                    || t.state == TaskState::Queued
                    || t.state == TaskState::Scheduled
                {
                    t.state = TaskState::Paused;
                    t.blocked_by = vec![swoop_domain::state::PauseReason::User];
                }
                if !self.queues.contains(&t.queue_id) {
                    t.queue_id = swoop_domain::QueueId::default_queue();
                }
                t.name = swoop_runtime::safety::sanitize_filename(&t.name);
                if swoop_runtime::safety::validate_destination_dir(&t.directory).is_err() {
                    t.directory = self.settings().storage.download_directory.clone();
                }
                t.touch();
                match self
                    .persist
                    .commit(crate::persist::PersistOp::Insert(Box::new(t.clone())))
                    .await
                {
                    Ok(()) => {
                        let is_new = !self.tasks.contains(&t.id);
                        self.tasks.insert(t.clone());
                        if is_new {
                            self.bus.publish(Event::TaskAdded(Box::new(t)));
                        } else {
                            self.bus.publish(Event::TaskUpdated(Box::new(t)));
                        }
                        bump("tasks", true);
                    }
                    Err(e) => {
                        bump("tasks", false);
                        report.errors.push(format!("task: {e}"));
                    }
                }
            }
        }
        self.admission.notify_one();
        Ok(report)
    }
}
