//! `devices` / `devices revoke ID`.

use osprey_domain::device::Device;

use crate::cli::{DevicesArgs, DevicesCommand};
use crate::error::CliResult;
use crate::output::{print_json, Table};
use crate::transport::Client;

pub async fn run(client: &Client, args: DevicesArgs, json: bool) -> CliResult<()> {
    match args.command {
        None => list(client, json).await,
        Some(DevicesCommand::Revoke(a)) => revoke(client, a.id, json).await,
    }
}

async fn list(client: &Client, json: bool) -> CliResult<()> {
    let value = client.get("/api/v1/devices").await?;
    let devices: Vec<Device> = serde_json::from_value(value)?;
    if json {
        print_json(&devices);
        return Ok(());
    }
    let mut t = Table::new(&["ID", "NAME", "KIND", "SCOPES", "LAST SEEN", "REVOKED"]);
    for d in &devices {
        t.push(vec![
            d.id.0.clone(),
            d.name.clone(),
            d.kind.clone(),
            d.scopes
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(","),
            d.last_seen_at
                .map(|m| m.to_datetime().to_rfc3339())
                .unwrap_or_else(|| "-".to_owned()),
            d.revoked.to_string(),
        ]);
    }
    t.print();
    Ok(())
}

async fn revoke(client: &Client, id: String, json: bool) -> CliResult<()> {
    let value = client.delete(&format!("/api/v1/devices/{id}")).await?;
    if json {
        print_json(&value);
    } else {
        println!("revoked {id}");
    }
    Ok(())
}
