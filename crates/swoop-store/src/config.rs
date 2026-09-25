//! Configuration entities stored as JSON documents: queues, categories, rules, schedules,
//! automation rules (+ consents and run log) and recipes.
//!
//! Each table keeps the full entity in a `json` column — the domain structs carry `#[serde
//! (default)]` on optional fields, so adding a field never needs a migration — plus the few
//! columns used for ordering.

use crate::error::{StoreError, StoreResult};
use crate::reader::{enum_from_str, enum_to_str, from_json, to_json};
use crate::Store;
use rusqlite::{params, Connection};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use swoop_domain::automation::{AutomationEvent, AutomationRule, AutomationRun, ConsentRecord};
use swoop_domain::category::Category;
use swoop_domain::queue::Queue;
use swoop_domain::rules::Rule;
use swoop_domain::schedule::Schedule;
use swoop_domain::{
    AutomationId, CategoryId, Millis, QueueId, RecipeId, RuleId, ScheduleId, TaskId,
};

/// Automation runs kept in the log.
pub const AUTOMATION_RUNS_KEEP: i64 = 1000;

/// A saved recipe. The services layer owns the `Recipe` type; the store keeps it as an opaque
/// JSON document with the columns needed to list it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecipeRecord {
    /// Recipe id.
    pub id: RecipeId,
    /// Display name (also inside `json`).
    pub name: String,
    /// The serialised recipe.
    pub json: serde_json::Value,
    /// Creation time.
    pub created_at: Millis,
    /// Last update time.
    pub updated_at: Millis,
}

/// Read every `json` document of a table in the given order.
fn list_docs<T: DeserializeOwned>(conn: &Connection, sql: &str) -> StoreResult<Vec<T>> {
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(from_json("json", &r?)?);
    }
    Ok(out)
}

fn delete_by_id(conn: &Connection, sql: &str, id: &str) -> StoreResult<bool> {
    Ok(conn.prepare_cached(sql)?.execute([id])? > 0)
}

impl Store {
    // ----- queues -----

    /// All queues ordered by sidebar position.
    pub async fn list_queues(&self) -> StoreResult<Vec<Queue>> {
        self.read(|conn| {
            list_docs(
                conn,
                "SELECT json FROM queues ORDER BY position ASC, created_at ASC",
            )
        })
        .await
    }

    /// Insert or replace a queue.
    pub async fn upsert_queue(&self, queue: &Queue) -> StoreResult<()> {
        let q = queue.clone();
        self.write("upsert_queue", move |conn| {
            conn.prepare_cached(
                "INSERT INTO queues (id, name, position, json, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name, \
                 position = excluded.position, json = excluded.json, \
                 updated_at = excluded.updated_at",
            )?
            .execute(params![
                q.id.as_str(),
                q.name,
                q.position,
                to_json(&q)?,
                q.created_at.0,
                q.updated_at.0
            ])?;
            Ok(())
        })
        .await
    }

    /// Delete a queue row (tasks referencing it are the services layer's job).
    pub async fn delete_queue(&self, id: &QueueId) -> StoreResult<bool> {
        let id = id.0.clone();
        self.write("delete_queue", move |conn| {
            delete_by_id(conn, "DELETE FROM queues WHERE id = ?1", &id)
        })
        .await
    }

    // ----- categories -----

    /// All categories ordered by position.
    pub async fn list_categories(&self) -> StoreResult<Vec<Category>> {
        self.read(|conn| {
            list_docs(
                conn,
                "SELECT json FROM categories ORDER BY position ASC, created_at ASC",
            )
        })
        .await
    }

    /// Insert or replace a category.
    pub async fn upsert_category(&self, category: &Category) -> StoreResult<()> {
        let c = category.clone();
        self.write("upsert_category", move |conn| {
            conn.prepare_cached(
                "INSERT INTO categories (id, name, position, json, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name, \
                 position = excluded.position, json = excluded.json, \
                 updated_at = excluded.updated_at",
            )?
            .execute(params![
                c.id.as_str(),
                c.name,
                c.position,
                to_json(&c)?,
                c.created_at.0,
                c.updated_at.0
            ])?;
            Ok(())
        })
        .await
    }

    /// Delete a category row.
    pub async fn delete_category(&self, id: &CategoryId) -> StoreResult<bool> {
        let id = id.0.clone();
        self.write("delete_category", move |conn| {
            delete_by_id(conn, "DELETE FROM categories WHERE id = ?1", &id)
        })
        .await
    }

    // ----- rules -----

    /// All organisation rules in evaluation order (priority, then creation time).
    pub async fn list_rules(&self) -> StoreResult<Vec<Rule>> {
        self.read(|conn| {
            list_docs(
                conn,
                "SELECT json FROM rules ORDER BY priority ASC, created_at ASC",
            )
        })
        .await
    }

    /// Insert or replace a rule.
    pub async fn upsert_rule(&self, rule: &Rule) -> StoreResult<()> {
        let r = rule.clone();
        self.write("upsert_rule", move |conn| {
            conn.prepare_cached(
                "INSERT INTO rules (id, name, priority, enabled, json, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name, \
                 priority = excluded.priority, enabled = excluded.enabled, \
                 json = excluded.json, updated_at = excluded.updated_at",
            )?
            .execute(params![
                r.id.as_str(),
                r.name,
                r.priority,
                r.enabled,
                to_json(&r)?,
                r.created_at.0,
                r.updated_at.0
            ])?;
            Ok(())
        })
        .await
    }

    /// Delete a rule row.
    pub async fn delete_rule(&self, id: &RuleId) -> StoreResult<bool> {
        let id = id.0.clone();
        self.write("delete_rule", move |conn| {
            delete_by_id(conn, "DELETE FROM rules WHERE id = ?1", &id)
        })
        .await
    }

    // ----- schedules -----

    /// All schedules, by name.
    pub async fn list_schedules(&self) -> StoreResult<Vec<Schedule>> {
        self.read(|conn| {
            list_docs(
                conn,
                "SELECT json FROM schedules ORDER BY name COLLATE NOCASE ASC, created_at ASC",
            )
        })
        .await
    }

    /// Insert or replace a schedule.
    pub async fn upsert_schedule(&self, schedule: &Schedule) -> StoreResult<()> {
        let s = schedule.clone();
        self.write("upsert_schedule", move |conn| {
            conn.prepare_cached(
                "INSERT INTO schedules (id, name, enabled, json, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name, \
                 enabled = excluded.enabled, json = excluded.json, \
                 updated_at = excluded.updated_at",
            )?
            .execute(params![
                s.id.as_str(),
                s.name,
                s.enabled,
                to_json(&s)?,
                s.created_at.0,
                s.updated_at.0
            ])?;
            Ok(())
        })
        .await
    }

    /// Delete a schedule row.
    pub async fn delete_schedule(&self, id: &ScheduleId) -> StoreResult<bool> {
        let id = id.0.clone();
        self.write("delete_schedule", move |conn| {
            delete_by_id(conn, "DELETE FROM schedules WHERE id = ?1", &id)
        })
        .await
    }

    // ----- automations -----

    /// All automation rules, by name.
    pub async fn list_automations(&self) -> StoreResult<Vec<AutomationRule>> {
        self.read(|conn| {
            list_docs(
                conn,
                "SELECT json FROM automations ORDER BY name COLLATE NOCASE ASC, created_at ASC",
            )
        })
        .await
    }

    /// Insert or replace an automation rule. Consents are untouched; the services layer
    /// re-validates them against the action hashes.
    pub async fn upsert_automation(&self, automation: &AutomationRule) -> StoreResult<()> {
        let a = automation.clone();
        self.write("upsert_automation", move |conn| {
            conn.prepare_cached(
                "INSERT INTO automations (id, name, enabled, json, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name, \
                 enabled = excluded.enabled, json = excluded.json, \
                 updated_at = excluded.updated_at",
            )?
            .execute(params![
                a.id.as_str(),
                a.name,
                a.enabled,
                to_json(&a)?,
                a.created_at.0,
                a.updated_at.0
            ])?;
            Ok(())
        })
        .await
    }

    /// Delete an automation rule and (by cascade) its consents.
    pub async fn delete_automation(&self, id: &AutomationId) -> StoreResult<bool> {
        let id = id.0.clone();
        self.write("delete_automation", move |conn| {
            delete_by_id(conn, "DELETE FROM automations WHERE id = ?1", &id)
        })
        .await
    }

    // ----- consents -----

    /// Replace every consent record of an automation with `records`. Records for other
    /// automations are rejected. The automation must exist.
    pub async fn grant_consents(
        &self,
        automation_id: &AutomationId,
        records: Vec<ConsentRecord>,
    ) -> StoreResult<()> {
        let id = automation_id.clone();
        self.write("grant_consents", move |conn| {
            if let Some(bad) = records.iter().find(|r| r.automation_id != id) {
                return Err(StoreError::Internal(format!(
                    "consent for {} passed to automation {}",
                    bad.automation_id, id
                )));
            }
            conn.prepare_cached("DELETE FROM consents WHERE automation_id = ?1")?
                .execute([id.as_str()])?;
            let mut ins = conn.prepare_cached(
                "INSERT INTO consents (automation_id, action_index, hash, granted_at) \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(automation_id, action_index) DO UPDATE SET hash = excluded.hash, \
                 granted_at = excluded.granted_at",
            )?;
            for r in &records {
                ins.execute(params![
                    r.automation_id.as_str(),
                    i64::from(r.action_index),
                    r.hash,
                    r.granted_at.0
                ])?;
            }
            Ok(())
        })
        .await
    }

    /// Consent records of an automation ordered by action index.
    pub async fn consents_for(
        &self,
        automation_id: &AutomationId,
    ) -> StoreResult<Vec<ConsentRecord>> {
        let id = automation_id.clone();
        self.read(move |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT automation_id, action_index, hash, granted_at FROM consents \
                 WHERE automation_id = ?1 ORDER BY action_index ASC",
            )?;
            let rows = stmt.query_map([id.as_str()], |r| {
                Ok(ConsentRecord {
                    automation_id: AutomationId(r.get(0)?),
                    action_index: r.get::<_, i64>(1)?.max(0) as u32,
                    hash: r.get(2)?,
                    granted_at: Millis(r.get(3)?),
                })
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .await
    }

    /// Remove every consent of an automation; returns how many were removed.
    pub async fn revoke_consents(&self, automation_id: &AutomationId) -> StoreResult<u64> {
        let id = automation_id.clone();
        self.write("revoke_consents", move |conn| {
            Ok(conn
                .prepare_cached("DELETE FROM consents WHERE automation_id = ?1")?
                .execute([id.as_str()])? as u64)
        })
        .await
    }

    // ----- automation runs -----

    /// Append a run record, keeping the newest [`AUTOMATION_RUNS_KEEP`] rows overall.
    pub async fn append_automation_run(&self, run: &AutomationRun) -> StoreResult<()> {
        let run = run.clone();
        self.write("append_automation_run", move |conn| {
            conn.prepare_cached(
                "INSERT INTO automation_runs (automation_id, task_id, event, at, success, \
                 message) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?
            .execute(params![
                run.automation_id.as_str(),
                run.task_id.as_ref().map(|t| t.as_str()),
                enum_to_str(&run.event)?,
                run.at.0,
                run.success,
                run.message,
            ])?;
            conn.prepare_cached(
                "DELETE FROM automation_runs WHERE id < \
                 (SELECT id FROM automation_runs ORDER BY id DESC LIMIT 1 OFFSET ?1)",
            )?
            .execute([AUTOMATION_RUNS_KEEP - 1])?;
            Ok(())
        })
        .await
    }

    /// Newest runs first, optionally for one automation. `limit == 0` = all.
    pub async fn automation_runs(
        &self,
        automation_id: Option<&AutomationId>,
        limit: u32,
    ) -> StoreResult<Vec<AutomationRun>> {
        let id = automation_id.map(|a| a.0.clone());
        self.read(move |conn| {
            let limit = if limit == 0 { -1 } else { i64::from(limit) };
            let mut stmt = conn.prepare_cached(
                "SELECT automation_id, task_id, event, at, success, message FROM automation_runs \
                 WHERE (?1 IS NULL OR automation_id = ?1) ORDER BY id DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![id, limit], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, bool>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })?;
            let mut out = Vec::new();
            for r in rows {
                let (automation_id, task_id, event, at, success, message) = r?;
                out.push(AutomationRun {
                    automation_id: AutomationId(automation_id),
                    task_id: task_id.map(TaskId),
                    event: enum_from_str::<AutomationEvent>("event", &event)?,
                    at: Millis(at),
                    success,
                    message,
                });
            }
            Ok(out)
        })
        .await
    }

    // ----- recipes -----

    /// All recipes by name.
    pub async fn list_recipes(&self) -> StoreResult<Vec<RecipeRecord>> {
        self.read(|conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT id, name, json, created_at, updated_at FROM recipes \
                 ORDER BY name COLLATE NOCASE ASC, created_at ASC",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            })?;
            let mut out = Vec::new();
            for r in rows {
                let (id, name, json, created_at, updated_at) = r?;
                out.push(RecipeRecord {
                    id: RecipeId(id),
                    name,
                    json: from_json("json", &json)?,
                    created_at: Millis(created_at),
                    updated_at: Millis(updated_at),
                });
            }
            Ok(out)
        })
        .await
    }

    /// Insert or replace a recipe.
    pub async fn upsert_recipe(&self, recipe: &RecipeRecord) -> StoreResult<()> {
        let r = recipe.clone();
        self.write("upsert_recipe", move |conn| {
            conn.prepare_cached(
                "INSERT INTO recipes (id, name, json, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5) \
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name, json = excluded.json, \
                 updated_at = excluded.updated_at",
            )?
            .execute(params![
                r.id.as_str(),
                r.name,
                to_json(&r.json)?,
                r.created_at.0,
                r.updated_at.0
            ])?;
            Ok(())
        })
        .await
    }

    /// Delete a recipe row.
    pub async fn delete_recipe(&self, id: &RecipeId) -> StoreResult<bool> {
        let id = id.0.clone();
        self.write("delete_recipe", move |conn| {
            delete_by_id(conn, "DELETE FROM recipes WHERE id = ?1", &id)
        })
        .await
    }
}
