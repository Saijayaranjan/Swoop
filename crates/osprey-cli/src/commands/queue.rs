//! `queue list|create|pause|resume|delete`.

use osprey_domain::queue::{Queue, QueueSummary};

use crate::cli::{QueueArgs, QueueCommand};
use crate::error::CliResult;
use crate::output::{print_json, speed, Table};
use crate::resolve::resolve_queue_id;
use crate::transport::Client;

pub async fn run(client: &Client, args: QueueArgs, json: bool) -> CliResult<()> {
    match args.command {
        QueueCommand::List => list(client, json).await,
        QueueCommand::Create(a) => create(client, a.name, a.max, json).await,
        QueueCommand::Pause(a) => pause(client, a.id, json).await,
        QueueCommand::Resume(a) => resume(client, a.id, json).await,
        QueueCommand::Delete(a) => delete(client, a.id, json).await,
    }
}

async fn list(client: &Client, json: bool) -> CliResult<()> {
    let value = client.get("/api/v1/queues/summaries").await?;
    let summaries: Vec<QueueSummary> = serde_json::from_value(value)?;
    let value = client.get("/api/v1/queues").await?;
    let queues: Vec<Queue> = serde_json::from_value(value)?;
    if json {
        print_json(&serde_json::json!({ "queues": queues, "summaries": summaries }));
        return Ok(());
    }
    let mut t = Table::new(&[
        "ID", "NAME", "MAX", "PAUSED", "ACTIVE", "WAITING", "DOWN", "UP",
    ]);
    for q in &queues {
        let s = summaries.iter().find(|s| s.queue_id == q.id);
        t.push(vec![
            q.id.0.clone(),
            q.name.clone(),
            if q.max_concurrent == 0 {
                "unlimited".to_owned()
            } else {
                q.max_concurrent.to_string()
            },
            q.paused.to_string(),
            s.map(|s| s.active.to_string()).unwrap_or_default(),
            s.map(|s| s.waiting.to_string()).unwrap_or_default(),
            s.map(|s| speed(s.download_speed)).unwrap_or_default(),
            s.map(|s| speed(s.upload_speed)).unwrap_or_default(),
        ]);
    }
    t.print();
    Ok(())
}

async fn create(client: &Client, name: String, max: u32, json: bool) -> CliResult<()> {
    let queue = Queue::new(name, max);
    let value = client
        .post("/api/v1/queues", serde_json::to_value(&queue)?)
        .await?;
    let created: Queue = serde_json::from_value(value)?;
    if json {
        print_json(&created);
    } else {
        println!("{}  {}", created.id, created.name);
    }
    Ok(())
}

async fn pause(client: &Client, id: String, json: bool) -> CliResult<()> {
    let qid = resolve_queue_id(client, &id).await?;
    let value = client
        .post_empty(&format!("/api/v1/queues/{qid}/pause"))
        .await?;
    let q: Queue = serde_json::from_value(value)?;
    if json {
        print_json(&q);
    } else {
        println!("{}  paused={}", q.id, q.paused);
    }
    Ok(())
}

async fn resume(client: &Client, id: String, json: bool) -> CliResult<()> {
    let qid = resolve_queue_id(client, &id).await?;
    let value = client
        .post_empty(&format!("/api/v1/queues/{qid}/resume"))
        .await?;
    let q: Queue = serde_json::from_value(value)?;
    if json {
        print_json(&q);
    } else {
        println!("{}  paused={}", q.id, q.paused);
    }
    Ok(())
}

async fn delete(client: &Client, id: String, json: bool) -> CliResult<()> {
    let qid = resolve_queue_id(client, &id).await?;
    let value = client.delete(&format!("/api/v1/queues/{qid}")).await?;
    if json {
        print_json(&value);
    } else {
        println!("deleted {qid}");
    }
    Ok(())
}
