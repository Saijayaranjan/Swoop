//! Site grabber sessions: start a crawl, forward its progress as events, persist finished
//! sessions, and turn discovered files into tasks.

use crate::api::{AddTaskResult, GrabberOptions, GrabberSession};
use crate::engine::Engine;
use swoop_domain::{DomainError, DomainResult, Event, NewTaskRequest};

impl Engine {
    pub(crate) async fn grabber_start_inner(
        &self,
        options: GrabberOptions,
    ) -> DomainResult<GrabberSession> {
        let session = self
            .grabber
            .start(options)
            .map_err(|e| DomainError::validation(e.message))?;
        let snapshot = session.snapshot();
        let this = self.this.clone();
        let mut rx = session.progress();
        let id = session.id.clone();
        tokio::spawn(async move {
            loop {
                let (pages, files, done) = *rx.borrow_and_update();
                let Some(engine) = this.upgrade() else {
                    break;
                };
                engine.bus.publish(Event::GrabberProgress {
                    session_id: id.clone(),
                    pages_crawled: pages,
                    files_found: files,
                    done,
                });
                if done {
                    if let Some(s) = engine.grabber.get(&id) {
                        if let Ok(json) = serde_json::to_value(s.snapshot()) {
                            if let Err(e) = engine.store.save_grabber_session(&id, &json).await {
                                tracing::debug!(error = %e, "grabber session not persisted");
                            }
                        }
                    }
                    break;
                }
                drop(engine);
                if rx.changed().await.is_err() {
                    break;
                }
            }
        });
        Ok(snapshot)
    }

    pub(crate) async fn grabber_status_inner(&self, id: String) -> DomainResult<GrabberSession> {
        if let Some(s) = self.grabber.get(&id) {
            return Ok(s.snapshot());
        }
        let stored = self.store.load_grabber_sessions().await?;
        stored
            .into_iter()
            .find(|(sid, _)| sid == &id)
            .and_then(|(_, json)| serde_json::from_value(json).ok())
            .ok_or_else(|| DomainError::not_found(format!("grabber session {id}")))
    }

    pub(crate) async fn grabber_cancel_inner(&self, id: String) -> DomainResult<()> {
        match self.grabber.get(&id) {
            Some(s) => {
                s.cancel();
                Ok(())
            }
            None => {
                if self.store.delete_grabber_session(&id).await? {
                    Ok(())
                } else {
                    Err(DomainError::not_found(format!("grabber session {id}")))
                }
            }
        }
    }

    pub(crate) async fn grabber_add_inner(
        &self,
        id: String,
        urls: Vec<String>,
        request: NewTaskRequest,
    ) -> DomainResult<Vec<AddTaskResult>> {
        let session = self.grabber_status_inner(id.clone()).await?;
        let mut out = Vec::new();
        for url in urls {
            let file = session.files.iter().find(|f| f.url == url);
            let mut req = request.clone();
            req.url = Some(url.clone());
            req.magnet = None;
            req.torrent_base64 = None;
            req.origin = "grabber".into();
            if req.referer_page.is_none() {
                req.referer_page = file.map(|f| f.found_on.clone());
            }
            if req.name.is_none() {
                req.name = file.map(|f| f.name.clone()).filter(|n| !n.is_empty());
            }
            match self.add_task_inner(req, None).await {
                Ok(r) => out.push(r),
                Err(e) => {
                    tracing::warn!(url = %swoop_runtime::redact::redact(&url), error = %e, "grabber add failed")
                }
            }
        }
        Ok(out)
    }

    pub(crate) async fn grabber_list_inner(&self) -> DomainResult<Vec<GrabberSession>> {
        let mut live = self.grabber.list();
        let ids: Vec<String> = live.iter().map(|s| s.id.clone()).collect();
        for (id, json) in self.store.load_grabber_sessions().await? {
            if ids.contains(&id) {
                continue;
            }
            if let Ok(s) = serde_json::from_value::<GrabberSession>(json) {
                live.push(s);
            }
        }
        live.sort_by_key(|s| std::cmp::Reverse(s.started_at));
        Ok(live)
    }
}
