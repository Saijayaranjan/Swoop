//! `diagnostics ID`.

use crate::cli::DiagnosticsArgs;
use crate::error::CliResult;
use crate::output::print_json;
use crate::resolve::resolve_task_id;
use crate::transport::Client;

pub async fn run(client: &Client, args: DiagnosticsArgs, json: bool) -> CliResult<()> {
    let id = resolve_task_id(client, &args.id).await?;
    if json {
        let value = client.get(&format!("/api/v1/tasks/{id}/diagnostics")).await?;
        print_json(&value);
    } else {
        let text = client
            .get_text(&format!("/api/v1/tasks/{id}/diagnostics.txt"))
            .await?;
        print!("{text}");
    }
    Ok(())
}
