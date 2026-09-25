//! `watch [ID]`: live progress over the `/api/v1/events` WebSocket.

use std::io::{IsTerminal, Write};

use futures::StreamExt;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::UnixStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use swoop_domain::task::Progress;

use crate::cli::WatchArgs;
use crate::error::CliError;
use crate::output::{eta, percent, speed};
use crate::resolve::resolve_task_id;
use crate::transport::{build_query, Client, Endpoint};

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

pub async fn run(client: &Client, args: WatchArgs) -> Result<(), CliError> {
    let single = args.id.is_some();
    let mut qs: Vec<(&str, Option<String>)> = Vec::new();
    if let Some(id) = &args.id {
        let full = resolve_task_id(client, id).await?;
        qs.push(("tasks", Some(full)));
    }

    match client.endpoint() {
        Endpoint::Socket { path, token } => {
            qs.push(("token", Some(token.clone())));
            let query = build_query(&qs);
            let stream = UnixStream::connect(path)
                .await
                .map_err(|_| CliError::not_running())?;
            let request = format!("ws://localhost/api/v1/events{query}")
                .into_client_request()
                .map_err(|e| CliError::Other(anyhow::anyhow!(e)))?;
            let (ws, _) = tokio_tungstenite::client_async(request, stream)
                .await
                .map_err(|e| CliError::Other(anyhow::anyhow!("websocket connect failed: {e}")))?;
            stream_events(ws, single).await
        }
        Endpoint::Remote { base, token } => {
            if let Some(t) = token {
                qs.push(("token", Some(t.clone())));
            }
            let query = build_query(&qs);
            let ws_base = base
                .replacen("https://", "wss://", 1)
                .replacen("http://", "ws://", 1);
            let url = format!("{}/api/v1/events{query}", ws_base.trim_end_matches('/'));
            let (ws, _) = tokio_tungstenite::connect_async(url)
                .await
                .map_err(|e| CliError::Other(anyhow::anyhow!("websocket connect failed: {e}")))?;
            stream_events(ws, single).await
        }
    }
}

async fn stream_events<S>(mut ws: WebSocketStream<S>, single: bool) -> Result<(), CliError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let is_tty = std::io::stdout().is_terminal();
    let mut last_len = 0usize;

    while let Some(msg) = ws.next().await {
        let msg = msg.map_err(|e| CliError::Other(anyhow::anyhow!(e)))?;
        let text = match msg {
            Message::Text(t) => t.to_string(),
            Message::Close(_) => break,
            _ => continue,
        };
        let frame: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let ty = frame.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match ty {
            "progress" => {
                if let Some(items) = frame.get("data").and_then(|v| v.as_array()) {
                    for item in items {
                        let task_id = item.get("task_id").and_then(|v| v.as_str()).unwrap_or("?");
                        let progress: Progress = item
                            .get("progress")
                            .cloned()
                            .and_then(|v| serde_json::from_value(v).ok())
                            .unwrap_or_default();
                        let line = format!(
                            "{}  {:>6}  {:>12}  ETA {}",
                            short_id(task_id),
                            percent(progress.percent()),
                            speed(progress.speed),
                            eta(progress.eta_seconds)
                        );
                        print_line(&line, is_tty && single, &mut last_len);
                    }
                }
            }
            "task_state_changed" => {
                let task_id = frame
                    .get("data")
                    .and_then(|d| d.get("task_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let to = frame
                    .get("data")
                    .and_then(|d| d.get("to"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let line = format!("{}  -> {}", short_id(task_id), to);
                if is_tty && single {
                    println!();
                }
                println!("{line}");
                last_len = 0;
                if single && matches!(to, "completed" | "failed" | "cancelled") {
                    break;
                }
            }
            "task_removed" if single => {
                println!("task removed");
                break;
            }
            _ => {}
        }
    }
    if is_tty && single {
        println!();
    }
    Ok(())
}

fn print_line(line: &str, redraw: bool, last_len: &mut usize) {
    if redraw {
        print!(
            "\r{:<width$}",
            line,
            width = (*last_len).max(line.chars().count())
        );
        let _ = std::io::stdout().flush();
        *last_len = line.chars().count();
    } else {
        println!("{line}");
    }
}
