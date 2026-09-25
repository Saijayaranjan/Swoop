//! Categories: CRUD and automatic assignment by extension / MIME.

use crate::engine::Engine;
use swoop_domain::category::Category;
use swoop_domain::{CategoryId, DomainError, DomainResult, Event, Millis, Task};

impl Engine {
    /// Category matching the task's extension or MIME type.
    pub(crate) fn auto_category(&self, task: &Task) -> Option<CategoryId> {
        let cats = self.categories.read();
        let ext = task.extension();
        if let Some(ext) = &ext {
            if let Some(c) = cats.iter().find(|c| c.matches_extension(ext)) {
                return Some(c.id.clone());
            }
        }
        if let Some(mime) = &task.mime {
            if let Some(c) = cats.iter().find(|c| c.matches_mime(mime)) {
                return Some(c.id.clone());
            }
        }
        None
    }

    /// Directory contribution of a category (`None` when it has no directory).
    pub(crate) fn category_dir(&self, id: &CategoryId) -> Option<std::path::PathBuf> {
        self.categories
            .read()
            .iter()
            .find(|c| &c.id == id)
            .and_then(|c| c.directory.clone())
            .filter(|d| !d.as_os_str().is_empty())
    }

    fn validate_category(c: &mut Category) -> DomainResult<()> {
        c.name = c.name.trim().to_owned();
        if c.name.is_empty() {
            return Err(DomainError::validation("category name must not be empty"));
        }
        c.extensions = c
            .extensions
            .iter()
            .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|e| !e.is_empty())
            .collect();
        if let Some(d) = &c.directory {
            let s = d.to_string_lossy();
            if s.contains("..") {
                return Err(DomainError::validation(
                    "category directory must not contain ..",
                ));
            }
        }
        Ok(())
    }

    pub(crate) async fn create_category_inner(&self, mut c: Category) -> DomainResult<Category> {
        Self::validate_category(&mut c)?;
        if self.categories.read().iter().any(|x| x.id == c.id) {
            return Err(DomainError::Conflict(format!("category {} exists", c.id)));
        }
        let now = Millis::now();
        c.created_at = now;
        c.updated_at = now;
        c.builtin = false;
        self.store.upsert_category(&c).await?;
        self.categories.write().push(c.clone());
        self.bus.publish(Event::CategoryUpdated(c.clone()));
        Ok(c)
    }

    pub(crate) async fn update_category_inner(&self, mut c: Category) -> DomainResult<Category> {
        Self::validate_category(&mut c)?;
        let existing = self
            .categories
            .read()
            .iter()
            .find(|x| x.id == c.id)
            .cloned()
            .ok_or_else(|| DomainError::not_found(format!("category {}", c.id)))?;
        c.builtin = existing.builtin;
        c.created_at = existing.created_at;
        c.updated_at = Millis::now();
        self.store.upsert_category(&c).await?;
        {
            let mut cats = self.categories.write();
            if let Some(slot) = cats.iter_mut().find(|x| x.id == c.id) {
                *slot = c.clone();
            }
        }
        self.bus.publish(Event::CategoryUpdated(c.clone()));
        Ok(c)
    }

    pub(crate) async fn delete_category_inner(&self, id: CategoryId) -> DomainResult<()> {
        let existing = self
            .categories
            .read()
            .iter()
            .find(|x| x.id == id)
            .cloned()
            .ok_or_else(|| DomainError::not_found(format!("category {id}")))?;
        if existing.builtin {
            return Err(DomainError::PermissionDenied(
                "built-in categories cannot be deleted".into(),
            ));
        }
        self.store.delete_category(&id).await?;
        self.categories.write().retain(|c| c.id != id);
        for tid in self.tasks.ids() {
            let Some(cell) = self.tasks.get(&tid) else {
                continue;
            };
            let snap = {
                let mut t = cell.lock();
                if t.category_id.as_ref() != Some(&id) {
                    continue;
                }
                t.category_id = None;
                t.touch();
                t.clone()
            };
            self.persist
                .send(crate::persist::PersistOp::Update(Box::new(snap.clone())));
            self.bus.publish(Event::TaskUpdated(Box::new(snap)));
        }
        self.bus.publish(Event::CategoryRemoved { category_id: id });
        Ok(())
    }
}
