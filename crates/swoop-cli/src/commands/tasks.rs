//! `add`, `list`, `status`, `pause`/`resume`/`retry`/`restart`/`cancel`, `remove`,
//! `pause-all`/`resume-all`.

use base64::Engine as _;
use swoop_domain::task::{Checksum, NewTaskRequest, Task, TaskOptions};
use swoop_domain::GlobalStats;
use swoop_services::api::{AddTaskResult, TaskRow};

use crate::cli::{AddArgs, IdsArgs, ListArgs, RemoveArgs, StatusArgs};
use crate::error::{CliError, CliResult};
use crate::output::{eta, percent, print_json, size_opt, speed, Table};
use crate::resolve::{resolve_queue_id, resolve_task_id};
use crate::transport::{build_query, Client};

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

enum SourceKind {
    Url(String),
    Magnet(String),
    TorrentFile(std::path::PathBuf),
}

fn classify(s: &str) -> SourceKind {
    if s.starts_with("magnet:") {
        SourceKind::Magnet(s.to_owned())
    } else if s.to_ascii_lowercase().ends_with(".torrent") && std::path::Path::new(s).exists() {
        SourceKind::TorrentFile(std::path::PathBuf::from(s))
    } else {
        SourceKind::Url(s.to_owned())
    }
}

pub async fn add(client: &Client, args: AddArgs, json: bool) -> CliResult<()> {
    let mut sources: Vec<SourceKind> = args.sources.iter().map(|s| classify(s)).collect();
    if let Some(t) = &args.torrent {
        sources.push(SourceKind::TorrentFile(t.clone()));
    }
    if let Some(m) = &args.magnet {
        sources.push(SourceKind::Magnet(m.clone()));
    }
    if sources.is_empty() {
        return Err(CliError::Usage(
            "add: give at least one URL, magnet URI, or .torrent file (or --torrent/--magnet)"
                .to_owned(),
        ));
    }

    let queue_id = match &args.queue {
        Some(q) => Some(resolve_queue_id(client, q).await?),
        None => None,
    };

    let mut options = TaskOptions {
        max_connections: args.connections,
        download_limit: args.limit,
        ..Default::default()
    };
    options.headers = args
        .headers
        .iter()
        .map(|h| (h.name.clone(), h.value.clone()))
        .collect();
    if let Some(c) = &args.checksum {
        options.checksum = Some(
            Checksum::parse(c)
                .ok_or_else(|| CliError::Usage(format!("invalid --checksum `{c}`")))?,
        );
    }

    let mut requests = Vec::with_capacity(sources.len());
    for src in sources {
        let mut req = NewTaskRequest {
            start: !args.paused,
            name: args.name.clone(),
            directory: args.dir.clone(),
            queue_id: queue_id.clone(),
            options: options.clone(),
            origin: "cli".to_owned(),
            ..Default::default()
        };
        match src {
            SourceKind::Url(u) => req.url = Some(u),
            SourceKind::Magnet(m) => req.magnet = Some(m),
            SourceKind::TorrentFile(path) => {
                let bytes = std::fs::read(&path)
                    .map_err(|e| CliError::Usage(format!("cannot read {}: {e}", path.display())))?;
                req.torrent_base64 = Some(base64::engine::general_purpose::STANDARD.encode(bytes));
                if req.name.is_none() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        req.name = Some(name.to_owned());
                    }
                }
            }
        }
        requests.push(req);
    }

    if requests.len() == 1 {
        let value = client
            .post("/api/v1/tasks", serde_json::to_value(&requests[0])?)
            .await?;
        let result: AddTaskResult = serde_json::from_value(value)?;
        if json {
            print_json(&result);
        } else {
            print_add_result(&result);
        }
    } else {
        let value = client
            .post("/api/v1/tasks/batch", serde_json::to_value(&requests)?)
            .await?;
        let results: Vec<AddTaskResult> = serde_json::from_value(value)?;
        if json {
            print_json(&results);
        } else {
            for r in &results {
                print_add_result(r);
            }
        }
    }
    Ok(())
}

fn print_add_result(result: &AddTaskResult) {
    if let Some(dup) = &result.duplicate {
        println!(
            "{}  {}  duplicate ({})",
            short_id(&result.task.id.0),
            result.task.name,
            dup.matched_by
        );
    } else {
        println!(
            "{}  {}  {}",
            short_id(&result.task.id.0),
            result.task.name,
            result.task.state.as_str()
        );
    }
}

pub async fn list(client: &Client, args: ListArgs, json: bool) -> CliResult<()> {
    let mut q: Vec<(&str, Option<String>)> = vec![("text", args.search.clone())];
    for s in &args.state {
        q.push(("state", Some(s.clone())));
    }
    if let Some(queue) = &args.queue {
        let qid = resolve_queue_id(client, queue).await?;
        q.push(("queue_id", Some(qid.0)));
    }
    if let Some(l) = args.limit {
        q.push(("limit", Some(l.to_string())));
    }
    let path = format!("/api/v1/tasks/rows{}", build_query(&q));
    let value = client.get(&path).await?;
    let rows: Vec<TaskRow> = serde_json::from_value(value)?;
    if json {
        print_json(&rows);
    } else {
        print_rows(&rows);
    }
    Ok(())
}

fn print_rows(rows: &[TaskRow]) {
    let mut t = Table::new(&["ID", "NAME", "STATE", "PROGRESS", "SIZE", "SPEED", "ETA"]);
    for r in rows {
        t.push(vec![
            short_id(&r.id.0),
            r.name.clone(),
            r.state.as_str().to_owned(),
            percent(r.progress.percent()),
            size_opt(r.progress.total),
            speed(r.progress.speed),
            eta(r.progress.eta_seconds),
        ]);
    }
    t.print();
}

pub async fn status(client: &Client, args: StatusArgs, json: bool) -> CliResult<()> {
    match args.id {
        None => {
            let value = client.get("/api/v1/stats").await?;
            let stats: GlobalStats = serde_json::from_value(value)?;
            if json {
                print_json(&stats);
            } else {
                print_stats(&stats);
            }
            Ok(())
        }
        Some(id) => {
            let full_id = resolve_task_id(client, &id).await?;
            let value = client.get(&format!("/api/v1/tasks/{full_id}")).await?;
            let task: Task = serde_json::from_value(value)?;
            if json {
                print_json(&task);
            } else {
                print_task_detail(&task);
            }
            Ok(())
        }
    }
}

fn print_stats(s: &GlobalStats) {
    println!("download   {}", speed(s.download_speed));
    println!("upload     {}", speed(s.upload_speed));
    println!("active     {}", s.active);
    println!("downloading {}", s.downloading);
    println!("seeding    {}", s.seeding);
    println!("queued     {}", s.queued);
    println!("scheduled  {}", s.scheduled);
    println!("paused     {}", s.paused);
    println!("completed today {}", s.completed_today);
    println!("failed today    {}", s.failed_today);
    println!("total tasks     {}", s.total_tasks);
    println!(
        "traffic mode    {}",
        serde_json::to_value(s.traffic_mode)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default()
    );
    println!(
        "limits          down {} / up {}",
        if s.download_limit == 0 {
            "unlimited".to_owned()
        } else {
            speed(s.download_limit)
        },
        if s.upload_limit == 0 {
            "unlimited".to_owned()
        } else {
            speed(s.upload_limit)
        }
    );
    println!("network available {}", s.network_available);
}

fn print_task_detail(t: &Task) {
    println!("id          {}", t.id);
    println!("name        {}", t.name);
    println!("kind        {}", t.kind.as_str());
    println!("state       {}", t.state.as_str());
    if let Some(d) = &t.domain() {
        println!("domain      {d}");
    }
    println!("directory   {}", t.directory.display());
    if let Some(fp) = &t.file_path {
        println!("file        {}", fp.display());
    }
    println!(
        "progress    {} of {} ({})",
        size_opt(Some(t.progress.downloaded)),
        size_opt(t.progress.total),
        percent(t.progress.percent())
    );
    println!("speed       {}", speed(t.progress.speed));
    println!("eta         {}", eta(t.progress.eta_seconds));
    println!("queue       {}", t.queue_id);
    println!("priority    {:?}", t.priority);
    if !t.tags.is_empty() {
        println!("tags        {}", t.tags.join(", "));
    }
    if let Some(err) = &t.error {
        println!("error       {:?}: {}", err.kind, err.message);
    }
    if !t.blocked_by.is_empty() {
        println!("blocked by  {:?}", t.blocked_by);
    }
}

/// Runs `action` (`pause`, `resume`, `retry`, `restart`, `cancel`) for each id, resolving id
/// prefixes first. Prints per-task results and returns an aggregate error if any failed.
pub async fn task_action(
    client: &Client,
    args: IdsArgs,
    action: &str,
    json: bool,
) -> CliResult<()> {
    let mut any_not_found = false;
    let mut any_error = false;
    let mut ok_results: Vec<Task> = Vec::new();

    for raw in &args.ids {
        let outcome = async {
            let full_id = resolve_task_id(client, raw).await?;
            let value = client
                .post_empty(&format!("/api/v1/tasks/{full_id}/{action}"))
                .await?;
            let task: Task = serde_json::from_value(value)?;
            Ok::<Task, CliError>(task)
        }
        .await;
        match outcome {
            Ok(task) => {
                if !json {
                    println!("{}  {}  {}", short_id(&task.id.0), task.name, task.state);
                }
                ok_results.push(task);
            }
            Err(e) => {
                any_error = true;
                if matches!(&e, CliError::NotFound(_))
                    || matches!(&e, CliError::Api { kind, .. } if kind == "not_found")
                {
                    any_not_found = true;
                }
                eprintln!("{raw}: {e}");
            }
        }
    }

    if json {
        print_json(&ok_results);
    }

    if any_not_found {
        Err(CliError::NotFound(
            "one or more tasks were not found".into(),
        ))
    } else if any_error {
        Err(CliError::Other(anyhow::anyhow!(
            "one or more actions failed"
        )))
    } else {
        Ok(())
    }
}

pub async fn remove(client: &Client, args: RemoveArgs, json: bool) -> CliResult<()> {
    let mut ids = Vec::with_capacity(args.ids.len());
    for raw in &args.ids {
        ids.push(resolve_task_id(client, raw).await?);
    }
    let body = serde_json::json!({ "ids": ids, "delete_file": args.delete_file });
    let value = client.post("/api/v1/tasks/remove", body).await?;
    if json {
        print_json(&value);
    } else if let Some(n) = value.get("removed").and_then(|v| v.as_u64()) {
        println!("removed {n} task(s)");
    }
    Ok(())
}

pub async fn pause_all(client: &Client, json: bool) -> CliResult<()> {
    let value = client.post_empty("/api/v1/tasks/pause-all").await?;
    report_count(&value, "paused", json)
}

pub async fn resume_all(client: &Client, json: bool) -> CliResult<()> {
    let value = client.post_empty("/api/v1/tasks/resume-all").await?;
    report_count(&value, "resumed", json)
}

fn report_count(value: &serde_json::Value, verb: &str, json: bool) -> CliResult<()> {
    if json {
        print_json(value);
    } else if let Some(n) = value.get("count").and_then(|v| v.as_u64()) {
        println!("{verb} {n} task(s)");
    }
    Ok(())
}
