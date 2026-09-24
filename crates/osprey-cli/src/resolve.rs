//! Resolving CLI arguments (endpoint, task/queue ids or name prefixes) against the running
//! engine.

use std::path::PathBuf;

use osprey_domain::QueueId;
use osprey_runtime::paths::AppPaths;
use osprey_services::api::TaskRow;

use crate::cli::Cli;
use crate::error::{CliError, CliResult};
use crate::transport::{Client, Endpoint};

/// Apply `--data-dir` as `OSPREY_DATA_DIR` so every path derived from [`AppPaths::resolve`]
/// (here and inside `osprey server`) agrees.
pub fn apply_data_dir_override(data_dir: &Option<PathBuf>) {
    if let Some(dir) = data_dir {
        std::env::set_var("OSPREY_DATA_DIR", dir);
    }
}

/// Build the client the rest of the CLI talks through, from `--url`/`--token` or the local
/// Unix socket + token file.
pub fn build_client(cli: &Cli) -> CliResult<Client> {
    if let Some(url) = &cli.url {
        return Ok(Client::new(Endpoint::Remote {
            base: url.clone(),
            token: cli.token.clone(),
        }));
    }
    let paths = AppPaths::resolve();
    let socket_path = cli.socket.clone().unwrap_or_else(|| paths.socket());
    let token = std::fs::read_to_string(paths.local_token_file())
        .map_err(|_| CliError::not_running())?
        .trim()
        .to_owned();
    Ok(Client::new(Endpoint::Socket {
        path: socket_path,
        token,
    }))
}

fn looks_like_full_id(s: &str) -> bool {
    s.len() == 36 && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// Resolve a task id or an unambiguous id prefix (as shown by `osprey list`) to a full task id.
pub async fn resolve_task_id(client: &Client, id_or_prefix: &str) -> CliResult<String> {
    if looks_like_full_id(id_or_prefix) {
        return Ok(id_or_prefix.to_owned());
    }
    let value = client.get("/api/v1/tasks/rows").await?;
    let rows: Vec<TaskRow> = serde_json::from_value(value)?;
    let matches: Vec<&TaskRow> = rows
        .iter()
        .filter(|r| r.id.0.starts_with(id_or_prefix))
        .collect();
    match matches.len() {
        0 => Err(CliError::NotFound(format!(
            "no task matching `{id_or_prefix}`"
        ))),
        1 => Ok(matches[0].id.0.clone()),
        _ => Err(CliError::Usage(format!(
            "`{id_or_prefix}` matches multiple tasks; use the full id"
        ))),
    }
}

/// Resolve a queue id or a queue name to a queue id.
pub async fn resolve_queue_id(client: &Client, id_or_name: &str) -> CliResult<QueueId> {
    let value = client.get("/api/v1/queues").await?;
    let queues: Vec<osprey_domain::queue::Queue> = serde_json::from_value(value)?;
    if let Some(q) = queues.iter().find(|q| q.id.0 == id_or_name) {
        return Ok(q.id.clone());
    }
    let matches: Vec<&osprey_domain::queue::Queue> = queues
        .iter()
        .filter(|q| q.name.eq_ignore_ascii_case(id_or_name))
        .collect();
    match matches.len() {
        1 => Ok(matches[0].id.clone()),
        0 => Ok(QueueId::from(id_or_name.to_owned())),
        _ => Err(CliError::Usage(format!(
            "`{id_or_name}` matches multiple queues; use the queue id"
        ))),
    }
}
