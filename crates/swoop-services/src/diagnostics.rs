//! Diagnostics: the per-task log, health score recomputation and the "Copy diagnostics" report.

use crate::api::{ConnectionInfo, TaskDiagnostics};
use crate::engine::Engine;
use std::fmt::Write as _;
use swoop_domain::events::LogLevel;
use swoop_domain::health::HealthScore;
use swoop_domain::{DomainError, DomainResult, Event, Millis, TaskId, TaskLogEntry};
use swoop_runtime::redact::redact;

impl Engine {
    /// Append a (redacted) log line for a task: ring, event, batched store write.
    pub(crate) fn task_log_line(&self, id: &TaskId, level: LogLevel, code: &str, message: String) {
        let entry = TaskLogEntry {
            task_id: id.clone(),
            at: Millis::now(),
            level,
            code: code.to_owned(),
            message: redact(&message),
        };
        match level {
            LogLevel::Error => tracing::warn!(task = %id, code, "{}", entry.message),
            LogLevel::Warn => tracing::info!(task = %id, code, "{}", entry.message),
            _ => tracing::debug!(task = %id, code, "{}", entry.message),
        }
        self.log_rings.push(entry.clone());
        self.log_batch.lock().push(entry.clone());
        self.bus.publish(Event::TaskLog(entry));
    }

    /// Flush batched log lines to the store (2 s ticker and shutdown).
    pub(crate) fn flush_log_batch(&self) {
        let lines: Vec<TaskLogEntry> = std::mem::take(&mut *self.log_batch.lock());
        if !lines.is_empty() {
            self.persist.send(crate::persist::PersistOp::Log(lines));
        }
    }

    /// The task's log tail, seeding the ring from the store on first access.
    pub(crate) async fn task_log_inner(
        &self,
        id: TaskId,
        limit: u32,
    ) -> DomainResult<Vec<TaskLogEntry>> {
        if !self.tasks.contains(&id) {
            return Err(DomainError::not_found(format!("task {id}")));
        }
        if !self.log_rings.has(&id) {
            let stored = self.store.task_log(&id, 0).await?;
            self.log_rings.seed(&id, stored);
        }
        Ok(self.log_rings.tail(&id, limit as usize))
    }

    /// Recompute the health score of every running task (5 s ticker).
    pub(crate) fn health_tick(&self) {
        for (id, run) in self.tasks.running() {
            let Some(cell) = self.tasks.get(&id) else {
                continue;
            };
            let snapshot = {
                let mut t = cell.lock();
                let inputs = {
                    let meter = run.meter.lock();
                    let accum = run.health.lock();
                    accum.inputs(&meter, &t, run.started.elapsed().as_secs())
                };
                let score = HealthScore::compute(&inputs);
                if score == t.health {
                    continue;
                }
                t.health = score;
                t.touch();
                t.clone()
            };
            self.persist
                .send(crate::persist::PersistOp::Update(Box::new(
                    snapshot.clone(),
                )));
            self.bus.publish(Event::TaskUpdated(Box::new(snapshot)));
        }
    }

    pub(crate) async fn diagnostics_inner(&self, id: TaskId) -> DomainResult<TaskDiagnostics> {
        let task = self
            .tasks
            .snapshot(&id)
            .ok_or_else(|| DomainError::not_found(format!("task {id}")))?;
        let log = self.task_log_inner(id.clone(), 0).await?;
        let connections = task
            .segment_map
            .as_ref()
            .map(|m| {
                m.segments
                    .iter()
                    .map(|s| ConnectionInfo {
                        segment_index: s.index,
                        source_index: s.source_index,
                        range_start: s.start,
                        range_end: s.end,
                        committed: s.committed,
                        speed: 0,
                        state: if s.is_done() { "done" } else { "pending" }.to_owned(),
                        remote_addr: task.stats.remote_addr.clone(),
                        http_version: task.stats.http_version.clone(),
                        retries: 0,
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(TaskDiagnostics {
            task: Some(task),
            log,
            connections,
            environment: self.environment_snapshot(),
            app_version: self.config.app_version.clone(),
            os: os_description(),
        })
    }

    /// Human-readable report for "Copy diagnostics".
    pub(crate) async fn diagnostics_text_inner(&self, id: TaskId) -> DomainResult<String> {
        let d = self.diagnostics_inner(id).await?;
        let Some(t) = &d.task else {
            return Err(DomainError::internal("diagnostics without task"));
        };
        let mut out = String::new();
        let _ = writeln!(
            out,
            "Swoop diagnostics — {}",
            Millis::now().to_datetime().to_rfc3339()
        );
        let _ = writeln!(
            out,
            "App {} · {} · {}",
            d.app_version,
            d.os,
            std::env::consts::ARCH
        );
        let _ = writeln!(out);
        let _ = writeln!(out, "Task {}", t.id);
        let _ = writeln!(out, "  name:      {}", t.name);
        let _ = writeln!(out, "  kind:      {}", t.kind.as_str());
        let _ = writeln!(out, "  state:     {}", t.state);
        if !t.blocked_by.is_empty() {
            let _ = writeln!(out, "  blocked:   {:?}", t.blocked_by);
        }
        let _ = writeln!(out, "  directory: {}", t.directory.display());
        if let Some(p) = &t.file_path {
            let _ = writeln!(out, "  file:      {}", p.display());
        }
        for (i, u) in t.source.urls().iter().enumerate() {
            let _ = writeln!(out, "  source[{i}]: {}", redact(u));
        }
        if let Some(final_url) = &t.stats.final_url {
            let _ = writeln!(out, "  final url: {}", redact(final_url));
        }
        let _ = writeln!(
            out,
            "  queue:     {}   category: {:?}",
            t.queue_id, t.category_id
        );
        let _ = writeln!(
            out,
            "  progress:  {} / {} bytes ({:.1}%)  speed {} B/s  eta {:?}",
            t.progress.downloaded,
            t.progress
                .total
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".into()),
            t.progress.percent().unwrap_or(0.0),
            t.progress.speed,
            t.progress.eta_seconds
        );
        let _ = writeln!(
            out,
            "  attempt:   {}   created {}   started {:?}   completed {:?}",
            t.attempt,
            t.created_at.to_datetime().to_rfc3339(),
            t.started_at.map(|m| m.to_datetime().to_rfc3339()),
            t.completed_at.map(|m| m.to_datetime().to_rfc3339())
        );
        if let Some(e) = &t.error {
            let _ = writeln!(
                out,
                "  error:     {:?} ({}) status={:?}",
                e.kind, e.message, e.status_code
            );
            if let Some(det) = &e.detail {
                let _ = writeln!(out, "             {}", redact(det));
            }
        }
        let _ = writeln!(out);
        let _ = writeln!(out, "Stats");
        let s = &t.stats;
        let _ = writeln!(out, "  avg {} B/s  peak {} B/s  retries {}  failed conns {}  reassigned {}  mirrors switched {}  drops {}  discarded {}",
            s.average_speed, s.peak_speed, s.retries, s.failed_connections, s.segments_reassigned,
            s.mirrors_switched, s.throughput_drops, s.bytes_discarded);
        let _ = writeln!(
            out,
            "  ranges {:?}  http {:?}  server {:?}  type {:?}  etag {:?}  remote {:?}",
            s.range_supported, s.http_version, s.server, s.content_type, s.etag, s.remote_addr
        );
        let h = &t.health;
        let _ = writeln!(out, "Health {} ({}) — stability {} consistency {} connections {} retry-pressure {} risk {} notes {:?}",
            h.score, h.label_key(), h.source_stability, h.throughput_consistency, h.connection_quality,
            h.retry_pressure, h.remaining_risk, h.notes);
        if !d.connections.is_empty() {
            let _ = writeln!(out);
            let _ = writeln!(out, "Segments ({})", d.connections.len());
            for c in &d.connections {
                let _ = writeln!(
                    out,
                    "  #{:<3} {:>12}-{:<12} committed {:>12} src {} {}",
                    c.segment_index,
                    c.range_start,
                    c.range_end,
                    c.committed,
                    c.source_index,
                    c.state
                );
            }
        }
        if let Some(tor) = &t.torrent {
            let _ = writeln!(out);
            let _ = writeln!(
                out,
                "Torrent {} files {} trackers {} peers {} seeds {} ratio {:.2}",
                tor.info_hash,
                tor.files.len(),
                tor.trackers.len(),
                tor.connected_peers,
                tor.connected_seeds,
                tor.ratio
            );
        }
        let _ = writeln!(out);
        let env = &d.environment;
        let _ = writeln!(
            out,
            "Environment network={} metered={} ac={} battery={:?} vpn={} at={}",
            env.network_available,
            env.metered,
            env.on_ac_power,
            env.battery_percent,
            env.vpn_active,
            env.at.to_datetime().to_rfc3339()
        );
        let settings = self.settings();
        let _ = writeln!(
            out,
            "Settings connections/task={} adaptive={} mode={:?} limits={:?} proxy={}",
            settings.network.connections_per_task,
            settings.network.adaptive_segmentation,
            settings.bandwidth.mode,
            settings.bandwidth.effective_limits(),
            settings.network.global_proxy.is_some()
        );
        let _ = writeln!(out);
        let _ = writeln!(out, "Log ({} lines)", d.log.len());
        for l in &d.log {
            let _ = writeln!(
                out,
                "  {} {:<5} {:<24} {}",
                l.at.to_datetime().format("%H:%M:%S%.3f"),
                format!("{:?}", l.level).to_uppercase(),
                l.code,
                l.message
            );
        }
        Ok(redact(&out))
    }
}

/// `macOS 15.1 (24B83)`-style description.
pub fn os_description() -> String {
    let name = sysinfo::System::name().unwrap_or_else(|| std::env::consts::OS.to_owned());
    let version = sysinfo::System::os_version().unwrap_or_default();
    format!("{name} {version}").trim().to_owned()
}
