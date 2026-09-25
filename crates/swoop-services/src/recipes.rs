//! Recipes: reusable download workflows stored as JSON documents.

use crate::api::{AddTaskResult, Recipe};
use crate::engine::Engine;
use swoop_domain::rules::RuleAction;
use swoop_domain::{DomainError, DomainResult, Millis, NewTaskRequest, RecipeId};
use swoop_store::RecipeRecord;

fn to_record(r: &Recipe) -> DomainResult<RecipeRecord> {
    Ok(RecipeRecord {
        id: r.id.clone(),
        name: r.name.clone(),
        json: serde_json::to_value(r).map_err(DomainError::internal)?,
        created_at: r.created_at,
        updated_at: r.updated_at,
    })
}

fn from_record(rec: RecipeRecord) -> Option<Recipe> {
    serde_json::from_value(rec.json).ok()
}

impl Engine {
    pub(crate) async fn list_recipes_inner(&self) -> DomainResult<Vec<Recipe>> {
        Ok(self
            .store
            .list_recipes()
            .await?
            .into_iter()
            .filter_map(from_record)
            .collect())
    }

    pub(crate) async fn save_recipe_inner(&self, mut r: Recipe) -> DomainResult<Recipe> {
        r.name = r.name.trim().to_owned();
        if r.name.is_empty() {
            return Err(DomainError::validation("recipe name must not be empty"));
        }
        if let Some(q) = &r.queue_id {
            if !self.queues.contains(q) {
                return Err(DomainError::not_found(format!("queue {q}")));
            }
        }
        if let Some(d) = &r.directory {
            let expanded = swoop_runtime::paths::AppPaths::expand_home(&d.to_string_lossy());
            swoop_runtime::safety::validate_destination_dir(&expanded)
                .map_err(|e| DomainError::validation(e.message))?;
            r.directory = Some(expanded);
        }
        let existing = self
            .store
            .list_recipes()
            .await?
            .into_iter()
            .find(|x| x.id == r.id);
        let now = Millis::now();
        r.created_at = existing.map(|e| e.created_at).unwrap_or(now);
        r.updated_at = now;
        self.store.upsert_recipe(&to_record(&r)?).await?;
        Ok(r)
    }

    pub(crate) async fn delete_recipe_inner(&self, id: RecipeId) -> DomainResult<()> {
        if !self.store.delete_recipe(&id).await? {
            return Err(DomainError::not_found(format!("recipe {id}")));
        }
        Ok(())
    }

    /// Merge a recipe into a request and add the task.
    pub(crate) async fn apply_recipe_inner(
        &self,
        id: RecipeId,
        mut request: NewTaskRequest,
    ) -> DomainResult<AddTaskResult> {
        let recipe = self
            .list_recipes_inner()
            .await?
            .into_iter()
            .find(|r| r.id == id)
            .ok_or_else(|| DomainError::not_found(format!("recipe {id}")))?;
        if request.queue_id.is_none() {
            request.queue_id = recipe.queue_id.clone();
        }
        if request.category_id.is_none() {
            request.category_id = recipe.category_id.clone();
        }
        if request.directory.is_none() {
            request.directory = recipe.directory.clone();
        }
        for t in &recipe.tags {
            if !request.tags.contains(t) {
                request.tags.push(t.clone());
            }
        }
        let base = request.options.clone();
        let mut opts = recipe.options.clone();
        // Explicit request values win over the recipe's.
        if base.max_connections.is_some() {
            opts.max_connections = base.max_connections;
        }
        if base.download_limit.is_some() {
            opts.download_limit = base.download_limit;
        }
        if base.checksum.is_some() {
            opts.checksum = base.checksum;
        }
        if !base.headers.is_empty() {
            opts.headers.extend(base.headers);
        }
        if base.credential.is_some() {
            opts.credential = base.credential;
        }
        request.options = opts;
        request.origin = format!("recipe:{}", recipe.id);
        for a in &recipe.rule_actions {
            match a {
                RuleAction::SetPriority { priority } => request.priority = Some(*priority),
                RuleAction::AddTags { tags } => {
                    for t in tags {
                        if !request.tags.contains(t) {
                            request.tags.push(t.clone());
                        }
                    }
                }
                _ => {}
            }
        }
        // Completion-time actions (and the recipe's automation) are looked up again by the
        // task's `recipe:<id>` origin when it finishes.
        self.add_task_inner(request, Some(recipe.rule_actions.clone()))
            .await
    }

    /// Completion-time work declared by the recipe a task came from.
    pub(crate) async fn recipe_post_actions(
        &self,
        task: &swoop_domain::Task,
    ) -> (Vec<RuleAction>, Option<swoop_domain::AutomationId>) {
        let Some(id) = task.origin.strip_prefix("recipe:") else {
            return (Vec::new(), None);
        };
        let Ok(recipes) = self.list_recipes_inner().await else {
            return (Vec::new(), None);
        };
        match recipes.into_iter().find(|r| r.id.as_str() == id) {
            Some(r) => (
                r.rule_actions
                    .iter()
                    .filter(|a| crate::rules::is_post_action(a))
                    .cloned()
                    .collect(),
                r.automation_id,
            ),
            None => (Vec::new(), None),
        }
    }
}
