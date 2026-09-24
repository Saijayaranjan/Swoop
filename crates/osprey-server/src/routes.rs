//! Route table and handlers. Every handler maps to exactly one [`osprey_services::EngineApi`]
//! method; required scopes are enforced centrally by [`crate::auth::guard`].

use crate::auth;
use crate::error::{ApiError, ApiResult};
use crate::extract::{ApiJson, Id, OptJson, QueryPairs};
use crate::state::{AppState, Caller, ClientIp};
use crate::{web, ws};
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::{from_fn_with_state, map_response};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Extension, Json, Router};
use osprey_domain::automation::AutomationRule;
use osprey_domain::category::Category;
use osprey_domain::device::{Device, Scope};
use osprey_domain::history::HistoryQuery;
use osprey_domain::queue::{Queue, TrafficMode};
use osprey_domain::rules::{Rule, RuleAction, RuleSubject};
use osprey_domain::schedule::{EnvironmentSnapshot, Schedule};
use osprey_domain::settings::Settings;
use osprey_domain::torrent::SeedingLimits;
use osprey_domain::{
    AutomationId, CategoryId, Checksum, ConflictPolicy, DeviceId, NewTaskRequest, PluginId,
    Priority, QueueId, RecipeId, RuleId, ScheduleId, TaskId, TaskState,
};
use osprey_services::{
    ExportBundle, FileSelection, GrabberOptions, ImportOptions, Recipe, TaskFilter, TaskPatch,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::PathBuf;
use tower_http::set_header::SetResponseHeaderLayer;

const JSON_LIMIT: usize = 1024 * 1024;
const TASK_ADD_LIMIT: usize = 10 * 1024 * 1024;

type St = State<AppState>;
type Who = Extension<Caller>;

pub(crate) fn build(state: AppState) -> Router {
    let big = || DefaultBodyLimit::max(TASK_ADD_LIMIT);
    let api = Router::new()
        .route("/healthz", get(healthz))
        .route("/api/v1/pair", post(pair))
        // ----- engine -----
        .route("/api/v1/info", get(info))
        .route("/api/v1/stats", get(stats))
        .route("/api/v1/dashboard", get(dashboard))
        .route("/api/v1/settings", get(get_settings).put(put_settings))
        .route("/api/v1/environment", post(environment))
        .route("/api/v1/traffic-mode", post(traffic_mode))
        .route("/api/v1/limits", post(limits))
        .route("/api/v1/optimize", post(optimize))
        .route("/api/v1/disk", get(disk))
        .route("/api/v1/logs", get(logs))
        .route("/api/v1/events", get(ws::events))
        // ----- tasks -----
        .route("/api/v1/tasks/probe", post(probe).layer(big()))
        .route(
            "/api/v1/tasks",
            get(list_tasks).post(add_task).layer(big()),
        )
        .route("/api/v1/tasks/batch", post(add_batch).layer(big()))
        .route("/api/v1/tasks/rows", get(list_rows))
        .route("/api/v1/tasks/remove", post(remove_tasks))
        .route("/api/v1/tasks/reorder", post(reorder_tasks))
        .route("/api/v1/tasks/pause-all", post(pause_all))
        .route("/api/v1/tasks/resume-all", post(resume_all))
        .route("/api/v1/tasks/retry-failed", post(retry_failed))
        .route("/api/v1/tasks/clear-completed", post(clear_completed))
        .route(
            "/api/v1/tasks/{id}",
            get(get_task).patch(patch_task).delete(delete_task),
        )
        .route("/api/v1/tasks/{id}/start", post(start_task))
        .route("/api/v1/tasks/{id}/pause", post(pause_task))
        .route("/api/v1/tasks/{id}/resume", post(resume_task))
        .route("/api/v1/tasks/{id}/restart", post(restart_task))
        .route("/api/v1/tasks/{id}/retry", post(retry_task))
        .route("/api/v1/tasks/{id}/cancel", post(cancel_task))
        .route("/api/v1/tasks/{id}/redownload", post(redownload))
        .route("/api/v1/tasks/{id}/verify", post(verify_task))
        .route("/api/v1/tasks/{id}/retry-segments", post(retry_segments))
        .route("/api/v1/tasks/{id}/retry-from-source", post(retry_from_source))
        .route("/api/v1/tasks/{id}/duplicate", post(duplicate_task))
        .route("/api/v1/tasks/{id}/resolve-duplicate", post(resolve_duplicate))
        .route("/api/v1/tasks/{id}/limit", post(task_limit))
        .route("/api/v1/tasks/{id}/connections", post(task_connections))
        .route("/api/v1/tasks/{id}/priority", post(task_priority))
        .route("/api/v1/tasks/{id}/log", get(task_log))
        .route("/api/v1/tasks/{id}/diagnostics", get(diagnostics))
        .route("/api/v1/tasks/{id}/diagnostics.txt", get(diagnostics_text))
        .route("/api/v1/tasks/{id}/file", get(task_file))
        // ----- torrents -----
        .route("/api/v1/tasks/{id}/peers", get(peers))
        .route("/api/v1/tasks/{id}/files", put(torrent_files))
        .route("/api/v1/tasks/{id}/sequential", post(sequential))
        .route("/api/v1/tasks/{id}/seeding", put(seeding))
        .route(
            "/api/v1/tasks/{id}/trackers",
            post(add_trackers).delete(remove_tracker),
        )
        .route("/api/v1/tasks/{id}/trackers/enable", post(enable_tracker))
        .route("/api/v1/tasks/{id}/reannounce", post(reannounce))
        .route("/api/v1/trackers/refresh", post(refresh_trackers))
        // ----- media -----
        .route("/api/v1/media/detect", post(detect_media))
        // ----- queues -----
        .route("/api/v1/queues", get(list_queues).post(create_queue))
        .route("/api/v1/queues/summaries", get(queue_summaries))
        .route("/api/v1/queues/reorder", post(reorder_queues))
        .route(
            "/api/v1/queues/{id}",
            put(update_queue).delete(delete_queue),
        )
        .route("/api/v1/queues/{id}/pause", post(pause_queue))
        .route("/api/v1/queues/{id}/resume", post(resume_queue))
        // ----- categories -----
        .route(
            "/api/v1/categories",
            get(list_categories).post(create_category),
        )
        .route(
            "/api/v1/categories/{id}",
            put(update_category).delete(delete_category),
        )
        // ----- rules -----
        .route("/api/v1/rules", get(list_rules).post(create_rule))
        .route("/api/v1/rules/test", post(test_rules))
        .route("/api/v1/rules/{id}", put(update_rule).delete(delete_rule))
        // ----- schedules -----
        .route(
            "/api/v1/schedules",
            get(list_schedules).post(create_schedule),
        )
        .route(
            "/api/v1/schedules/{id}",
            put(update_schedule).delete(delete_schedule),
        )
        // ----- automations -----
        .route(
            "/api/v1/automations",
            get(list_automations).post(create_automation),
        )
        .route("/api/v1/automations/runs", get(automation_runs))
        .route(
            "/api/v1/automations/{id}",
            put(update_automation).delete(delete_automation),
        )
        .route("/api/v1/automations/{id}/run", post(run_automation))
        // ----- recipes -----
        .route("/api/v1/recipes", get(list_recipes).post(create_recipe))
        .route(
            "/api/v1/recipes/{id}",
            put(update_recipe).delete(delete_recipe),
        )
        .route(
            "/api/v1/recipes/{id}/apply",
            post(apply_recipe).layer(big()),
        )
        // ----- history -----
        .route("/api/v1/history", get(history).delete(clear_history))
        .route("/api/v1/history/count", get(history_count))
        .route("/api/v1/history/delete", post(delete_history))
        // ----- devices -----
        .route("/api/v1/devices", get(list_devices))
        .route(
            "/api/v1/devices/pairing",
            post(start_pairing).delete(cancel_pairing),
        )
        .route(
            "/api/v1/devices/{id}",
            delete(revoke_device).patch(rename_device),
        )
        .route("/api/v1/audit", get(audit))
        // ----- grabber -----
        .route("/api/v1/grabber", get(grabber_list).post(grabber_start))
        .route(
            "/api/v1/grabber/{id}",
            get(grabber_status).delete(grabber_cancel),
        )
        .route("/api/v1/grabber/{id}/add", post(grabber_add).layer(big()))
        // ----- archives (local only) -----
        .route("/api/v1/archives/list", post(archive_list))
        .route("/api/v1/archives/extract", post(archive_extract))
        // ----- import / export / updates / plugins -----
        .route("/api/v1/export", get(export))
        .route("/api/v1/import", post(import).layer(big()))
        .route("/api/v1/updates/check", post(check_updates))
        .route("/api/v1/updates/download", post(download_update))
        .route("/api/v1/plugins", get(list_plugins))
        .route("/api/v1/plugins/{id}/enable", post(enable_plugin))
        .route("/api/v1/plugins/{id}", delete(uninstall_plugin))
        .route_layer(from_fn_with_state(state.clone(), auth::guard))
        .layer(map_response(api_cache_headers));

    Router::new()
        .merge(api)
        .fallback(web::fallback)
        .layer(DefaultBodyLimit::max(JSON_LIMIT))
        .layer(from_fn_with_state(state.clone(), auth::origin_check))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "default-src 'self'; connect-src 'self' ws: wss:; img-src 'self' data:; \
                 object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'",
            ),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            http::HeaderName::from_static("cross-origin-resource-policy"),
            HeaderValue::from_static("same-origin"),
        ))
        .with_state(state)
}

/// API responses are never cacheable (they may contain tokens, paths, …).
async fn api_cache_headers(mut resp: Response) -> Response {
    resp.headers_mut()
        .entry(header::CACHE_CONTROL)
        .or_insert(HeaderValue::from_static("no-store"));
    resp
}

fn no_content() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

fn created<T: serde::Serialize>(v: T) -> Response {
    (StatusCode::CREATED, Json(v)).into_response()
}

fn count(n: u32) -> Json<Value> {
    Json(json!({ "count": n }))
}

// ---------------------------------------------------------------------------------------------
// Remote-safety helpers
// ---------------------------------------------------------------------------------------------

/// Normalise a task request from an API caller. Remote devices cannot make Osprey open a
/// downloaded file automatically, and their tasks are tagged with a `remote` origin.
fn sanitize_request(c: &Caller, mut r: NewTaskRequest) -> NewTaskRequest {
    if c.trusted {
        if r.origin.is_empty() {
            r.origin = "api".into();
        }
    } else {
        r.options.open_when_done = false;
        r.origin = "remote".into();
    }
    r
}

fn refuse_exec_automation(c: &Caller, a: &AutomationRule) -> ApiResult<()> {
    if !c.trusted && a.actions.iter().any(|x| x.requires_consent()) {
        return Err(ApiError::forbidden(
            "automations that run commands or scripts can only be created or changed on this computer",
        ));
    }
    Ok(())
}

/// Ids of automations containing code-executing actions.
async fn exec_automation_ids(st: &AppState) -> ApiResult<HashSet<AutomationId>> {
    Ok(st
        .engine
        .list_automations()
        .await?
        .into_iter()
        .filter(|a| a.actions.iter().any(|x| x.requires_consent()))
        .map(|a| a.id)
        .collect())
}

/// Remote callers may not wire rules/recipes to automations that execute code.
async fn refuse_exec_links(
    st: &AppState,
    c: &Caller,
    actions: &[RuleAction],
    automation: Option<&AutomationId>,
) -> ApiResult<()> {
    if c.trusted {
        return Ok(());
    }
    let linked: Vec<&AutomationId> = actions
        .iter()
        .filter_map(|a| match a {
            RuleAction::RunAutomation { automation_id } => Some(automation_id),
            _ => None,
        })
        .chain(automation)
        .collect();
    if linked.is_empty() {
        return Ok(());
    }
    let exec = exec_automation_ids(st).await?;
    if linked.iter().any(|id| exec.contains(*id)) {
        return Err(ApiError::forbidden(
            "cannot link an automation that runs commands or scripts from a remote device",
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Health + pairing
// ---------------------------------------------------------------------------------------------

async fn healthz(State(st): St) -> Json<Value> {
    Json(json!({ "status": "ok", "version": st.engine.info().version }))
}

#[derive(Deserialize)]
struct PairBody {
    code: String,
    #[serde(default)]
    device_name: String,
    #[serde(default)]
    device_kind: String,
}

async fn pair(
    State(st): St,
    Extension(ClientIp(ip)): Extension<ClientIp>,
    ApiJson(b): ApiJson<PairBody>,
) -> ApiResult<Json<Value>> {
    if b.code.len() > 32 || b.device_name.chars().count() > 100 || b.device_kind.len() > 32 {
        return Err(ApiError::validation("field too long"));
    }
    let name = if b.device_name.trim().is_empty() {
        "Unnamed device".to_owned()
    } else {
        b.device_name
    };
    match st
        .engine
        .complete_pairing(b.code, name, b.device_kind, ip)
        .await
    {
        Ok((device, token)) => Ok(Json(json!({ "device": device, "token": token }))),
        Err(osprey_domain::DomainError::PermissionDenied(m)) if m.contains("locked") => {
            Err(ApiError {
                message: m,
                ..ApiError::rate_limited(60)
            })
        }
        Err(osprey_domain::DomainError::PermissionDenied(_)) => {
            Err(ApiError::unauthorized("invalid or expired pairing code"))
        }
        Err(e) => Err(e.into()),
    }
}

// ---------------------------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------------------------

async fn info(State(st): St) -> Response {
    Json(st.engine.info()).into_response()
}

async fn stats(State(st): St) -> Response {
    Json(st.engine.global_stats()).into_response()
}

async fn dashboard(State(st): St) -> ApiResult<Response> {
    Ok(Json(st.engine.dashboard().await?).into_response())
}

async fn get_settings(State(st): St) -> Response {
    Json(st.engine.settings().as_ref().clone()).into_response()
}

async fn put_settings(State(st): St, ApiJson(s): ApiJson<Settings>) -> ApiResult<Response> {
    Ok(Json(st.engine.update_settings(s).await?).into_response())
}

async fn environment(
    State(st): St,
    ApiJson(env): ApiJson<EnvironmentSnapshot>,
) -> ApiResult<Response> {
    st.engine.update_environment(env).await;
    Ok(no_content())
}

#[derive(Deserialize)]
struct ModeBody {
    mode: TrafficMode,
}

async fn traffic_mode(State(st): St, ApiJson(b): ApiJson<ModeBody>) -> ApiResult<Response> {
    st.engine.set_traffic_mode(b.mode).await?;
    Ok(no_content())
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct LimitsBody {
    download: u64,
    upload: u64,
}

async fn limits(State(st): St, ApiJson(b): ApiJson<LimitsBody>) -> ApiResult<Response> {
    st.engine.set_global_limits(b.download, b.upload).await?;
    Ok(no_content())
}

async fn optimize(State(st): St) -> ApiResult<Response> {
    Ok(Json(st.engine.optimize().await?).into_response())
}

async fn disk(State(st): St, q: QueryPairs) -> ApiResult<Response> {
    let path = q.string("path").map(PathBuf::from);
    Ok(Json(st.engine.disk_info(path).await?).into_response())
}

async fn logs(State(st): St, q: QueryPairs) -> ApiResult<Response> {
    let limit = q.num::<u32>("limit")?.unwrap_or(200).min(5000);
    let level = q.string("level");
    Ok(Json(st.engine.recent_logs(limit, level).await?).into_response())
}

// ---------------------------------------------------------------------------------------------
// Tasks
// ---------------------------------------------------------------------------------------------

fn task_filter(q: &QueryPairs) -> ApiResult<TaskFilter> {
    let states = q
        .all("state")
        .iter()
        .map(|s| QueryPairs::enum_value::<TaskState>("state", s))
        .collect::<ApiResult<Vec<_>>>()?;
    let kinds = q
        .all("kind")
        .iter()
        .map(|s| QueryPairs::enum_value("kind", s))
        .collect::<ApiResult<Vec<_>>>()?;
    Ok(TaskFilter {
        text: q.string("text"),
        states,
        kinds,
        queue_id: q.string("queue_id").map(QueueId),
        category_id: q.string("category_id").map(CategoryId),
        domain: q.string("domain"),
        tag: q.string("tag"),
        smart: q.string("smart"),
        created_since: q.num::<i64>("created_since")?.map(osprey_domain::Millis),
        min_size: q.num("min_size")?,
        max_size: q.num("max_size")?,
        sort: q.opt_enum("sort")?.unwrap_or_default(),
        descending: q.bool("desc")?.or(q.bool("descending")?).unwrap_or(false),
        limit: q.num("limit")?.unwrap_or(0),
        offset: q.num("offset")?.unwrap_or(0),
    })
}

async fn probe(
    State(st): St,
    Extension(c): Who,
    ApiJson(r): ApiJson<NewTaskRequest>,
) -> ApiResult<Response> {
    Ok(Json(st.engine.probe(sanitize_request(&c, r)).await?).into_response())
}

async fn add_task(
    State(st): St,
    Extension(c): Who,
    ApiJson(r): ApiJson<NewTaskRequest>,
) -> ApiResult<Response> {
    Ok(created(st.engine.add_task(sanitize_request(&c, r)).await?))
}

async fn add_batch(
    State(st): St,
    Extension(c): Who,
    ApiJson(rs): ApiJson<Vec<NewTaskRequest>>,
) -> ApiResult<Response> {
    if rs.len() > 1000 {
        return Err(ApiError::validation("at most 1000 tasks per batch"));
    }
    let rs = rs.into_iter().map(|r| sanitize_request(&c, r)).collect();
    Ok(Json(st.engine.add_tasks(rs).await?).into_response())
}

async fn list_tasks(State(st): St, q: QueryPairs) -> ApiResult<Response> {
    Ok(Json(st.engine.list_tasks(task_filter(&q)?).await?).into_response())
}

async fn list_rows(State(st): St, q: QueryPairs) -> ApiResult<Response> {
    Ok(Json(st.engine.list_rows(task_filter(&q)?).await?).into_response())
}

async fn get_task(State(st): St, Id(id): Id) -> ApiResult<Response> {
    Ok(Json(st.engine.get_task(TaskId(id)).await?).into_response())
}

async fn patch_task(
    State(st): St,
    Extension(c): Who,
    Id(id): Id,
    ApiJson(mut p): ApiJson<TaskPatch>,
) -> ApiResult<Response> {
    let id = TaskId(id);
    if !c.trusted {
        if let Some(opts) = p.options.as_mut() {
            // A remote device may not switch on "open when done".
            let current = st.engine.get_task(id.clone()).await?;
            opts.open_when_done = current.options.open_when_done && opts.open_when_done;
        }
    }
    Ok(Json(st.engine.update_task(id, p).await?).into_response())
}

async fn delete_task(State(st): St, Id(id): Id, q: QueryPairs) -> ApiResult<Response> {
    let delete_file = q.bool("delete_file")?.unwrap_or(false);
    st.engine.remove_task(TaskId(id), delete_file).await?;
    Ok(no_content())
}

#[derive(Deserialize)]
struct RemoveBody {
    ids: Vec<TaskId>,
    #[serde(default)]
    delete_file: bool,
}

async fn remove_tasks(State(st): St, ApiJson(b): ApiJson<RemoveBody>) -> ApiResult<Response> {
    let n = st.engine.remove_tasks(b.ids, b.delete_file).await?;
    Ok(Json(json!({ "removed": n })).into_response())
}

#[derive(Deserialize)]
struct ReorderBody {
    ids: Vec<TaskId>,
    #[serde(default)]
    after: Option<TaskId>,
}

async fn reorder_tasks(State(st): St, ApiJson(b): ApiJson<ReorderBody>) -> ApiResult<Response> {
    st.engine.reorder_tasks(b.ids, b.after).await?;
    Ok(no_content())
}

macro_rules! task_action {
    ($name:ident, $method:ident) => {
        async fn $name(State(st): St, Id(id): Id) -> ApiResult<Response> {
            Ok(Json(st.engine.$method(TaskId(id)).await?).into_response())
        }
    };
}

task_action!(start_task, start_task);
task_action!(pause_task, pause_task);
task_action!(resume_task, resume_task);
task_action!(restart_task, restart_task);
task_action!(retry_task, retry_task);
task_action!(cancel_task, cancel_task);
task_action!(redownload, redownload);
task_action!(retry_segments, retry_failed_segments);
task_action!(duplicate_task, duplicate_task);

#[derive(Deserialize, Default)]
#[serde(default)]
struct VerifyBody {
    /// `"sha256:<hex>"` or `{"algorithm": "...", "value": "..."}`.
    checksum: Option<Value>,
}

async fn verify_task(
    State(st): St,
    Id(id): Id,
    OptJson(b): OptJson<VerifyBody>,
) -> ApiResult<Response> {
    let checksum = match b.checksum {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(
            Checksum::parse(&s).ok_or_else(|| ApiError::validation("unrecognised checksum"))?,
        ),
        Some(v) => Some(
            serde_json::from_value::<Checksum>(v)
                .map_err(|e| ApiError::validation(format!("checksum: {e}")))?,
        ),
    };
    Ok(Json(st.engine.verify_task(TaskId(id), checksum).await?).into_response())
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct UrlBody {
    url: Option<String>,
}

async fn retry_from_source(
    State(st): St,
    Id(id): Id,
    OptJson(b): OptJson<UrlBody>,
) -> ApiResult<Response> {
    let url = b.url.filter(|u| !u.trim().is_empty());
    Ok(Json(st.engine.retry_from_source(TaskId(id), url).await?).into_response())
}

#[derive(Deserialize)]
struct PolicyBody {
    policy: ConflictPolicy,
}

async fn resolve_duplicate(
    State(st): St,
    Id(id): Id,
    ApiJson(b): ApiJson<PolicyBody>,
) -> ApiResult<Response> {
    Ok(Json(st.engine.resolve_duplicate(TaskId(id), b.policy).await?).into_response())
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct TaskLimitBody {
    download: Option<u64>,
    upload: Option<u64>,
}

async fn task_limit(
    State(st): St,
    Id(id): Id,
    ApiJson(b): ApiJson<TaskLimitBody>,
) -> ApiResult<Response> {
    Ok(Json(
        st.engine
            .set_task_limit(TaskId(id), b.download, b.upload)
            .await?,
    )
    .into_response())
}

#[derive(Deserialize)]
struct ConnectionsBody {
    connections: u8,
}

async fn task_connections(
    State(st): St,
    Id(id): Id,
    ApiJson(b): ApiJson<ConnectionsBody>,
) -> ApiResult<Response> {
    Ok(Json(
        st.engine
            .set_task_connections(TaskId(id), b.connections)
            .await?,
    )
    .into_response())
}

#[derive(Deserialize)]
struct PriorityBody {
    priority: Priority,
}

async fn task_priority(
    State(st): St,
    Id(id): Id,
    ApiJson(b): ApiJson<PriorityBody>,
) -> ApiResult<Response> {
    Ok(Json(st.engine.set_task_priority(TaskId(id), b.priority).await?).into_response())
}

async fn task_log(State(st): St, Id(id): Id, q: QueryPairs) -> ApiResult<Response> {
    let limit = q.num::<u32>("limit")?.unwrap_or(200).min(10_000);
    Ok(Json(st.engine.task_log(TaskId(id), limit).await?).into_response())
}

async fn diagnostics(State(st): St, Id(id): Id) -> ApiResult<Response> {
    Ok(Json(st.engine.diagnostics(TaskId(id)).await?).into_response())
}

async fn diagnostics_text(State(st): St, Id(id): Id) -> ApiResult<Response> {
    let text = st.engine.diagnostics_text(TaskId(id)).await?;
    Ok((
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )],
        text,
    )
        .into_response())
}

async fn pause_all(State(st): St) -> ApiResult<Json<Value>> {
    Ok(count(st.engine.pause_all().await?))
}
async fn resume_all(State(st): St) -> ApiResult<Json<Value>> {
    Ok(count(st.engine.resume_all().await?))
}
async fn retry_failed(State(st): St) -> ApiResult<Json<Value>> {
    Ok(count(st.engine.retry_all_failed().await?))
}
async fn clear_completed(State(st): St) -> ApiResult<Json<Value>> {
    Ok(count(st.engine.clear_completed().await?))
}

/// `Content-Disposition: attachment` with an ASCII fallback and an RFC 5987 UTF-8 name.
fn content_disposition(name: &str) -> HeaderValue {
    let ascii: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let encoded = percent_encoding::utf8_percent_encode(name, percent_encoding::NON_ALPHANUMERIC);
    HeaderValue::from_str(&format!(
        "attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}"
    ))
    .unwrap_or_else(|_| HeaderValue::from_static("attachment"))
}

async fn task_file(State(st): St, Id(id): Id) -> ApiResult<Response> {
    let task = st.engine.get_task(TaskId(id)).await?;
    if task.state != TaskState::Completed {
        return Err(ApiError::conflict("the download is not complete"));
    }
    let path = task
        .file_path
        .clone()
        .ok_or_else(|| ApiError::conflict("the download has no file"))?;
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| ApiError::not_found("the downloaded file no longer exists"))?;
    let meta = file
        .metadata()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if !meta.is_file() {
        return Err(ApiError::conflict(
            "the download is a folder; fetch individual files locally",
        ));
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| task.name.clone());
    let mime = mime_guess::from_path(&path).first_or_octet_stream();
    let stream = tokio_util::io::ReaderStream::with_capacity(file, 64 * 1024);
    let mut resp = Response::new(Body::from_stream(stream));
    let h = resp.headers_mut();
    if let Ok(v) = HeaderValue::from_str(mime.essence_str()) {
        h.insert(header::CONTENT_TYPE, v);
    }
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(meta.len()));
    h.insert(header::CONTENT_DISPOSITION, content_disposition(&name));
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("sandbox; default-src 'none'"),
    );
    Ok(resp)
}

// ----- torrents -----

async fn peers(State(st): St, Id(id): Id) -> ApiResult<Response> {
    Ok(Json(st.engine.torrent_peers(TaskId(id)).await?).into_response())
}

async fn torrent_files(
    State(st): St,
    Id(id): Id,
    ApiJson(sel): ApiJson<Vec<FileSelection>>,
) -> ApiResult<Response> {
    Ok(Json(st.engine.set_torrent_files(TaskId(id), sel).await?).into_response())
}

#[derive(Deserialize)]
struct SequentialBody {
    sequential: bool,
}

async fn sequential(
    State(st): St,
    Id(id): Id,
    ApiJson(b): ApiJson<SequentialBody>,
) -> ApiResult<Response> {
    Ok(Json(
        st.engine
            .set_torrent_sequential(TaskId(id), b.sequential)
            .await?,
    )
    .into_response())
}

async fn seeding(
    State(st): St,
    Id(id): Id,
    ApiJson(l): ApiJson<SeedingLimits>,
) -> ApiResult<Response> {
    Ok(Json(st.engine.set_seeding_limits(TaskId(id), l).await?).into_response())
}

#[derive(Deserialize)]
struct TrackersBody {
    trackers: Vec<String>,
}

async fn add_trackers(
    State(st): St,
    Id(id): Id,
    ApiJson(b): ApiJson<TrackersBody>,
) -> ApiResult<Response> {
    Ok(Json(st.engine.add_trackers(TaskId(id), b.trackers).await?).into_response())
}

#[derive(Deserialize)]
struct TrackerBody {
    tracker: String,
    #[serde(default = "yes")]
    enabled: bool,
}

fn yes() -> bool {
    true
}

async fn remove_tracker(
    State(st): St,
    Id(id): Id,
    ApiJson(b): ApiJson<TrackerBody>,
) -> ApiResult<Response> {
    Ok(Json(st.engine.remove_tracker(TaskId(id), b.tracker).await?).into_response())
}

async fn enable_tracker(
    State(st): St,
    Id(id): Id,
    ApiJson(b): ApiJson<TrackerBody>,
) -> ApiResult<Response> {
    Ok(Json(
        st.engine
            .set_tracker_enabled(TaskId(id), b.tracker, b.enabled)
            .await?,
    )
    .into_response())
}

async fn reannounce(State(st): St, Id(id): Id) -> ApiResult<Response> {
    st.engine.reannounce(TaskId(id)).await?;
    Ok(no_content())
}

async fn refresh_trackers(State(st): St) -> ApiResult<Json<Value>> {
    let n = st.engine.refresh_tracker_list().await?;
    Ok(Json(json!({ "applied": n })))
}

// ----- media -----

#[derive(Deserialize)]
struct DetectBody {
    url: String,
    #[serde(default)]
    page_url: Option<String>,
}

async fn detect_media(State(st): St, ApiJson(b): ApiJson<DetectBody>) -> ApiResult<Response> {
    Ok(Json(st.engine.detect_media(b.url, b.page_url).await?).into_response())
}

// ---------------------------------------------------------------------------------------------
// Queues, categories, rules, schedules, automations, recipes
// ---------------------------------------------------------------------------------------------

async fn list_queues(State(st): St) -> ApiResult<Response> {
    Ok(Json(st.engine.list_queues().await?).into_response())
}
async fn queue_summaries(State(st): St) -> ApiResult<Response> {
    Ok(Json(st.engine.queue_summaries().await?).into_response())
}
async fn create_queue(State(st): St, ApiJson(q): ApiJson<Queue>) -> ApiResult<Response> {
    Ok(created(st.engine.create_queue(q).await?))
}
async fn update_queue(
    State(st): St,
    Id(id): Id,
    ApiJson(mut q): ApiJson<Queue>,
) -> ApiResult<Response> {
    q.id = QueueId(id);
    Ok(Json(st.engine.update_queue(q).await?).into_response())
}
async fn delete_queue(State(st): St, Id(id): Id, q: QueryPairs) -> ApiResult<Response> {
    let move_to = q.string("move_to").map(QueueId);
    st.engine.delete_queue(QueueId(id), move_to).await?;
    Ok(no_content())
}
async fn pause_queue(State(st): St, Id(id): Id) -> ApiResult<Response> {
    Ok(Json(st.engine.pause_queue(QueueId(id)).await?).into_response())
}
async fn resume_queue(State(st): St, Id(id): Id) -> ApiResult<Response> {
    Ok(Json(st.engine.resume_queue(QueueId(id)).await?).into_response())
}

#[derive(Deserialize)]
struct IdsBody<T> {
    ids: Vec<T>,
}

async fn reorder_queues(
    State(st): St,
    ApiJson(b): ApiJson<IdsBody<QueueId>>,
) -> ApiResult<Response> {
    st.engine.reorder_queues(b.ids).await?;
    Ok(no_content())
}

async fn list_categories(State(st): St) -> ApiResult<Response> {
    Ok(Json(st.engine.list_categories().await?).into_response())
}
async fn create_category(State(st): St, ApiJson(c): ApiJson<Category>) -> ApiResult<Response> {
    Ok(created(st.engine.create_category(c).await?))
}
async fn update_category(
    State(st): St,
    Id(id): Id,
    ApiJson(mut c): ApiJson<Category>,
) -> ApiResult<Response> {
    c.id = CategoryId(id);
    Ok(Json(st.engine.update_category(c).await?).into_response())
}
async fn delete_category(State(st): St, Id(id): Id) -> ApiResult<Response> {
    st.engine.delete_category(CategoryId(id)).await?;
    Ok(no_content())
}

async fn list_rules(State(st): St) -> ApiResult<Response> {
    Ok(Json(st.engine.list_rules().await?).into_response())
}
async fn create_rule(
    State(st): St,
    Extension(c): Who,
    ApiJson(r): ApiJson<Rule>,
) -> ApiResult<Response> {
    refuse_exec_links(&st, &c, &r.actions, None).await?;
    Ok(created(st.engine.create_rule(r).await?))
}
async fn update_rule(
    State(st): St,
    Extension(c): Who,
    Id(id): Id,
    ApiJson(mut r): ApiJson<Rule>,
) -> ApiResult<Response> {
    r.id = RuleId(id);
    refuse_exec_links(&st, &c, &r.actions, None).await?;
    Ok(Json(st.engine.update_rule(r).await?).into_response())
}
async fn delete_rule(State(st): St, Id(id): Id) -> ApiResult<Response> {
    st.engine.delete_rule(RuleId(id)).await?;
    Ok(no_content())
}
async fn test_rules(State(st): St, ApiJson(s): ApiJson<RuleSubject>) -> ApiResult<Response> {
    let out: Vec<Value> = st
        .engine
        .test_rules(s)
        .await?
        .into_iter()
        .map(|(rule, actions)| json!({ "rule": rule, "actions": actions }))
        .collect();
    Ok(Json(out).into_response())
}

async fn list_schedules(State(st): St) -> ApiResult<Response> {
    Ok(Json(st.engine.list_schedules().await?).into_response())
}
async fn create_schedule(State(st): St, ApiJson(s): ApiJson<Schedule>) -> ApiResult<Response> {
    Ok(created(st.engine.create_schedule(s).await?))
}
async fn update_schedule(
    State(st): St,
    Id(id): Id,
    ApiJson(mut s): ApiJson<Schedule>,
) -> ApiResult<Response> {
    s.id = ScheduleId(id);
    Ok(Json(st.engine.update_schedule(s).await?).into_response())
}
async fn delete_schedule(State(st): St, Id(id): Id) -> ApiResult<Response> {
    st.engine.delete_schedule(ScheduleId(id)).await?;
    Ok(no_content())
}

async fn list_automations(State(st): St) -> ApiResult<Response> {
    Ok(Json(st.engine.list_automations().await?).into_response())
}
async fn create_automation(
    State(st): St,
    Extension(c): Who,
    ApiJson(a): ApiJson<AutomationRule>,
) -> ApiResult<Response> {
    refuse_exec_automation(&c, &a)?;
    Ok(created(st.engine.create_automation(a).await?))
}
async fn update_automation(
    State(st): St,
    Extension(c): Who,
    Id(id): Id,
    ApiJson(mut a): ApiJson<AutomationRule>,
) -> ApiResult<Response> {
    a.id = AutomationId(id);
    refuse_exec_automation(&c, &a)?;
    Ok(Json(st.engine.update_automation(a).await?).into_response())
}
async fn delete_automation(State(st): St, Id(id): Id) -> ApiResult<Response> {
    st.engine.delete_automation(AutomationId(id)).await?;
    Ok(no_content())
}
async fn automation_runs(State(st): St, q: QueryPairs) -> ApiResult<Response> {
    let id = q.string("id").map(AutomationId);
    let limit = q.num::<u32>("limit")?.unwrap_or(100).min(5000);
    Ok(Json(st.engine.automation_runs(id, limit).await?).into_response())
}

#[derive(Deserialize)]
struct RunBody {
    task_id: TaskId,
}

async fn run_automation(
    State(st): St,
    Extension(c): Who,
    Id(id): Id,
    ApiJson(b): ApiJson<RunBody>,
) -> ApiResult<Response> {
    let id = AutomationId(id);
    if !c.trusted && exec_automation_ids(&st).await?.contains(&id) {
        return Err(ApiError::forbidden(
            "automations that run commands or scripts can only be run manually on this computer",
        ));
    }
    Ok(Json(st.engine.run_automation(id, b.task_id).await?).into_response())
}

async fn list_recipes(State(st): St) -> ApiResult<Response> {
    Ok(Json(st.engine.list_recipes().await?).into_response())
}
async fn save_recipe_checked(st: &AppState, c: &Caller, mut r: Recipe) -> ApiResult<Recipe> {
    if !c.trusted {
        r.options.open_when_done = false;
    }
    refuse_exec_links(st, c, &r.rule_actions, r.automation_id.as_ref()).await?;
    Ok(st.engine.save_recipe(r).await?)
}
async fn create_recipe(
    State(st): St,
    Extension(c): Who,
    ApiJson(r): ApiJson<Recipe>,
) -> ApiResult<Response> {
    Ok(created(save_recipe_checked(&st, &c, r).await?))
}
async fn update_recipe(
    State(st): St,
    Extension(c): Who,
    Id(id): Id,
    ApiJson(mut r): ApiJson<Recipe>,
) -> ApiResult<Response> {
    r.id = RecipeId(id);
    Ok(Json(save_recipe_checked(&st, &c, r).await?).into_response())
}
async fn delete_recipe(State(st): St, Id(id): Id) -> ApiResult<Response> {
    st.engine.delete_recipe(RecipeId(id)).await?;
    Ok(no_content())
}
async fn apply_recipe(
    State(st): St,
    Extension(c): Who,
    Id(id): Id,
    ApiJson(r): ApiJson<NewTaskRequest>,
) -> ApiResult<Response> {
    Ok(created(
        st.engine
            .apply_recipe(RecipeId(id), sanitize_request(&c, r))
            .await?,
    ))
}

// ---------------------------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------------------------

fn history_query(q: &QueryPairs) -> ApiResult<HistoryQuery> {
    Ok(HistoryQuery {
        text: q.string("text"),
        domain: q.string("domain"),
        state: q.opt_enum("state")?,
        kind: q.opt_enum("kind")?,
        category_id: q.string("category_id").map(CategoryId),
        queue_id: q.string("queue_id").map(QueueId),
        since: q.num::<i64>("since")?.map(osprey_domain::Millis),
        until: q.num::<i64>("until")?.map(osprey_domain::Millis),
        min_size: q.num("min_size")?,
        max_size: q.num("max_size")?,
        tag: q.string("tag"),
        sort: q.opt_enum("sort")?.unwrap_or_default(),
        descending: q.bool("descending")?.or(q.bool("desc")?).unwrap_or(false),
        limit: q.num("limit")?.unwrap_or(0),
        offset: q.num("offset")?.unwrap_or(0),
    })
}

async fn history(State(st): St, q: QueryPairs) -> ApiResult<Response> {
    Ok(Json(st.engine.history(history_query(&q)?).await?).into_response())
}
async fn history_count(State(st): St, q: QueryPairs) -> ApiResult<Json<Value>> {
    Ok(count(st.engine.history_count(history_query(&q)?).await?))
}
async fn delete_history(
    State(st): St,
    ApiJson(b): ApiJson<IdsBody<TaskId>>,
) -> ApiResult<Json<Value>> {
    Ok(count(st.engine.delete_history(b.ids).await?))
}
async fn clear_history(State(st): St) -> ApiResult<Json<Value>> {
    Ok(count(st.engine.clear_history().await?))
}

// ---------------------------------------------------------------------------------------------
// Devices
// ---------------------------------------------------------------------------------------------

async fn list_devices(State(st): St) -> ApiResult<Response> {
    Ok(Json(st.engine.list_devices().await?).into_response())
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PairingBody {
    scopes: Vec<Scope>,
}

async fn start_pairing(
    State(st): St,
    OptJson(b): OptJson<PairingBody>,
) -> ApiResult<Response> {
    let mut info = st.engine.start_pairing(b.scopes).await?;
    if info.tls_fingerprint.is_none() {
        info.tls_fingerprint = st.cfg.tls_fingerprint.clone();
    }
    Ok(Json(info).into_response())
}

async fn cancel_pairing(State(st): St) -> ApiResult<Response> {
    st.engine.cancel_pairing().await?;
    Ok(no_content())
}

async fn revoke_device(State(st): St, Id(id): Id) -> ApiResult<Response> {
    st.engine.revoke_device(DeviceId(id)).await?;
    Ok(no_content())
}

#[derive(Deserialize)]
struct RenameBody {
    name: String,
}

async fn rename_device(
    State(st): St,
    Id(id): Id,
    ApiJson(b): ApiJson<RenameBody>,
) -> ApiResult<Json<Device>> {
    if b.name.trim().is_empty() || b.name.chars().count() > 100 {
        return Err(ApiError::validation("name must be 1–100 characters"));
    }
    Ok(Json(st.engine.rename_device(DeviceId(id), b.name).await?))
}

async fn audit(State(st): St, q: QueryPairs) -> ApiResult<Response> {
    let limit = q.num::<u32>("limit")?.unwrap_or(200).min(10_000);
    Ok(Json(st.engine.audit_log(limit).await?).into_response())
}

// ---------------------------------------------------------------------------------------------
// Grabber, archives, import/export, updates, plugins
// ---------------------------------------------------------------------------------------------

async fn grabber_start(
    State(st): St,
    ApiJson(o): ApiJson<GrabberOptions>,
) -> ApiResult<Response> {
    Ok(created(st.engine.grabber_start(o).await?))
}
async fn grabber_list(State(st): St) -> ApiResult<Response> {
    Ok(Json(st.engine.grabber_list().await?).into_response())
}
async fn grabber_status(State(st): St, Id(id): Id) -> ApiResult<Response> {
    Ok(Json(st.engine.grabber_status(id).await?).into_response())
}
async fn grabber_cancel(State(st): St, Id(id): Id) -> ApiResult<Response> {
    st.engine.grabber_cancel(id).await?;
    Ok(no_content())
}

#[derive(Deserialize)]
struct GrabberAddBody {
    urls: Vec<String>,
    #[serde(default)]
    request: NewTaskRequest,
}

async fn grabber_add(
    State(st): St,
    Extension(c): Who,
    Id(id): Id,
    ApiJson(b): ApiJson<GrabberAddBody>,
) -> ApiResult<Response> {
    let req = sanitize_request(&c, b.request);
    Ok(Json(st.engine.grabber_add(id, b.urls, req).await?).into_response())
}

#[derive(Deserialize)]
struct ArchiveListBody {
    path: PathBuf,
}

async fn archive_list(
    State(st): St,
    ApiJson(b): ApiJson<ArchiveListBody>,
) -> ApiResult<Response> {
    Ok(Json(st.engine.archive_list(b.path).await?).into_response())
}

#[derive(Deserialize)]
struct ArchiveExtractBody {
    path: PathBuf,
    #[serde(default)]
    entries: Option<Vec<String>>,
    destination: PathBuf,
}

async fn archive_extract(
    State(st): St,
    ApiJson(b): ApiJson<ArchiveExtractBody>,
) -> ApiResult<Json<Value>> {
    let n = st
        .engine
        .archive_extract(b.path, b.entries, b.destination)
        .await?;
    Ok(Json(json!({ "extracted": n })))
}

async fn export(State(st): St, q: QueryPairs) -> ApiResult<Response> {
    let tasks = q.bool("tasks")?.unwrap_or(true);
    let history = q.bool("history")?.unwrap_or(false);
    Ok(Json(st.engine.export(tasks, history).await?).into_response())
}

#[derive(Deserialize)]
struct ImportBody {
    bundle: ExportBundle,
    #[serde(default)]
    options: ImportOptions,
}

async fn import(
    State(st): St,
    Extension(c): Who,
    ApiJson(b): ApiJson<ImportBody>,
) -> ApiResult<Response> {
    if !c.trusted && b.options.automations {
        for a in &b.bundle.automations {
            refuse_exec_automation(&c, a)?;
        }
    }
    Ok(Json(st.engine.import(b.bundle, b.options).await?).into_response())
}

async fn check_updates(State(st): St) -> ApiResult<Response> {
    Ok(Json(st.engine.check_for_updates().await?).into_response())
}

async fn download_update(State(st): St) -> ApiResult<Json<Value>> {
    let path = st.engine.download_update().await?;
    Ok(Json(json!({ "path": path })))
}

async fn list_plugins(State(st): St) -> ApiResult<Response> {
    Ok(Json(st.engine.list_plugins().await?).into_response())
}

#[derive(Deserialize)]
struct PluginEnableBody {
    enabled: bool,
    #[serde(default)]
    permissions: Vec<String>,
}

async fn enable_plugin(
    State(st): St,
    Id(id): Id,
    ApiJson(b): ApiJson<PluginEnableBody>,
) -> ApiResult<Response> {
    Ok(Json(
        st.engine
            .set_plugin_enabled(PluginId(id), b.enabled, b.permissions)
            .await?,
    )
    .into_response())
}

async fn uninstall_plugin(State(st): St, Id(id): Id) -> ApiResult<Response> {
    st.engine.uninstall_plugin(PluginId(id)).await?;
    Ok(no_content())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disposition_is_safe() {
        let v = content_disposition("rés\"umé\r\n.pdf");
        let s = v.to_str().unwrap();
        assert!(s.starts_with("attachment; filename=\""));
        assert!(!s.contains('\r') && !s.contains('\n'));
        assert!(s.contains("filename*=UTF-8''r%C3%A9s%22um%C3%A9%0D%0A%2Epdf"));
    }
}
