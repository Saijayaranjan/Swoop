//! `limit` and `mode`.

use osprey_domain::GlobalStats;

use crate::cli::{LimitArgs, ModeArgs};
use crate::error::CliResult;
use crate::output::print_json;
use crate::transport::Client;

pub async fn limit(client: &Client, args: LimitArgs, json: bool) -> CliResult<()> {
    let (current_down, current_up) = {
        let value = client.get("/api/v1/stats").await?;
        let stats: GlobalStats = serde_json::from_value(value)?;
        (stats.download_limit, stats.upload_limit)
    };
    let download = args.down.unwrap_or(current_down);
    let upload = args.up.unwrap_or(current_up);
    let body = serde_json::json!({ "download": download, "upload": upload });
    client.post("/api/v1/limits", body).await?;
    if json {
        print_json(&serde_json::json!({ "download": download, "upload": upload }));
    } else {
        let fmt = |v: u64| {
            if v == 0 {
                "unlimited".to_owned()
            } else {
                crate::output::speed(v)
            }
        };
        println!("download {}", fmt(download));
        println!("upload   {}", fmt(upload));
    }
    Ok(())
}

pub async fn mode(client: &Client, args: ModeArgs, json: bool) -> CliResult<()> {
    let body = serde_json::json!({ "mode": args.mode.as_str() });
    client.post("/api/v1/traffic-mode", body).await?;
    if json {
        print_json(&serde_json::json!({ "mode": args.mode.as_str() }));
    } else {
        println!("traffic mode set to {}", args.mode.as_str());
    }
    Ok(())
}
