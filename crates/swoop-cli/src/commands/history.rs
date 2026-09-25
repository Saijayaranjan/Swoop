//! `history`.

use swoop_domain::history::HistoryEntry;

use crate::cli::HistoryArgs;
use crate::error::CliResult;
use crate::output::{print_json, size_opt, Table};
use crate::transport::build_query;
use crate::transport::Client;

pub async fn run(client: &Client, args: HistoryArgs, json: bool) -> CliResult<()> {
    let q: Vec<(&str, Option<String>)> = vec![
        ("text", args.search.clone()),
        ("limit", args.limit.map(|l| l.to_string())),
    ];
    let path = format!("/api/v1/history{}", build_query(&q));
    let value = client.get(&path).await?;
    let entries: Vec<HistoryEntry> = serde_json::from_value(value)?;
    if json {
        print_json(&entries);
        return Ok(());
    }
    let mut t = Table::new(&["NAME", "STATE", "SIZE", "DOMAIN", "FINISHED", "DESTINATION"]);
    for e in &entries {
        t.push(vec![
            e.name.clone(),
            e.state.as_str().to_owned(),
            size_opt(e.size),
            e.domain.clone(),
            e.finished_at.to_datetime().to_rfc3339(),
            e.destination.display().to_string(),
        ]);
    }
    t.print();
    Ok(())
}
