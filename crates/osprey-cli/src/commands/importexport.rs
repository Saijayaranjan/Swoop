//! `export` / `import`.

use osprey_services::api::{ExportBundle, ImportOptions, ImportReport};

use crate::cli::{ExportArgs, ImportArgs};
use crate::error::{CliError, CliResult};
use crate::transport::Client;

/// Always prints raw JSON to stdout (regardless of `--json`) so `osprey export > file.json`
/// produces a bundle `osprey import` can read back.
pub async fn export(client: &Client, args: ExportArgs) -> CliResult<()> {
    let q: Vec<(&str, Option<String>)> = vec![
        ("tasks", Some(args.tasks.to_string())),
        ("history", Some(args.history.to_string())),
    ];
    let path = format!("/api/v1/export{}", crate::transport::build_query(&q));
    let value = client.get(&path).await?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

pub async fn import(client: &Client, args: ImportArgs, json: bool) -> CliResult<()> {
    let text = std::fs::read_to_string(&args.file)
        .map_err(|e| CliError::Usage(format!("cannot read {}: {e}", args.file.display())))?;
    let bundle: ExportBundle = serde_json::from_str(&text).map_err(|e| {
        CliError::Usage(format!("{} is not a valid export bundle: {e}", args.file.display()))
    })?;
    let options = ImportOptions {
        settings: bundle.settings.is_some(),
        queues: !bundle.queues.is_empty(),
        categories: !bundle.categories.is_empty(),
        rules: !bundle.rules.is_empty(),
        schedules: !bundle.schedules.is_empty(),
        automations: !bundle.automations.is_empty(),
        tasks: !bundle.tasks.is_empty(),
        history: !bundle.history.is_empty(),
        recipes: !bundle.recipes.is_empty(),
        overwrite: args.overwrite,
    };
    let body = serde_json::json!({ "bundle": bundle, "options": options });
    let value = client.post("/api/v1/import", body).await?;
    let report: ImportReport = serde_json::from_value(value)?;
    if json {
        crate::output::print_json(&report);
        return Ok(());
    }
    for (k, v) in &report.imported {
        println!("imported {k}: {v}");
    }
    for (k, v) in &report.skipped {
        println!("skipped  {k}: {v}");
    }
    for e in &report.errors {
        eprintln!("error: {e}");
    }
    if !report.errors.is_empty() {
        return Err(CliError::Other(anyhow::anyhow!("import completed with errors")));
    }
    Ok(())
}
