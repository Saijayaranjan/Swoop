//! `GET /api/v1/events` — the WebSocket event stream (see `docs/api/websocket.md`).

use crate::extract::QueryPairs;
use crate::state::{AppState, Caller};
use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use axum::Extension;
use futures::{SinkExt, StreamExt};
use osprey_domain::device::Scope;
use osprey_domain::Event;
use osprey_services::TaskRow;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::{Duration, Instant};
use tokio::sync::broadcast::error::RecvError;

const PING_EVERY: Duration = Duration::from_secs(30);
const IDLE_CLOSE: Duration = Duration::from_secs(90);
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_INBOUND: usize = 64 * 1024;
const MAX_SUBSCRIBED_TASKS: usize = 10_000;

#[derive(Clone, Debug, Default)]
pub(crate) struct Filter {
    pub high_volume: bool,
    pub tasks: Option<HashSet<String>>,
}

pub(crate) async fn events(
    State(st): State<AppState>,
    Extension(caller): Extension<Caller>,
    q: QueryPairs,
    ws: WebSocketUpgrade,
) -> Response {
    let high_volume = q.bool("high_volume").ok().flatten().unwrap_or(true);
    let tasks = parse_tasks(q.all("tasks"));
    let filter = Filter { high_volume, tasks };
    ws.max_message_size(MAX_INBOUND)
        .max_frame_size(MAX_INBOUND)
        .on_upgrade(move |socket| run(socket, st, caller, filter))
}

fn parse_tasks(ids: Vec<String>) -> Option<HashSet<String>> {
    if ids.is_empty() {
        None
    } else {
        Some(ids.into_iter().take(MAX_SUBSCRIBED_TASKS).collect())
    }
}

/// Project an event to the `(type, data)` a given client may see, or `None` to drop it.
pub(crate) fn project(ev: &Event, caller: &Caller, filter: &Filter) -> Option<(&'static str, Value)> {
    let remote = !caller.trusted;
    match ev {
        Event::PlatformAction { .. } | Event::SettingsChanged(_) if remote => return None,
        Event::DeviceUpdated(_)
        | Event::DeviceRemoved { .. }
        | Event::PairingStarted { .. }
        | Event::PairingCompleted { .. }
            if remote && !caller.has(Scope::Admin) =>
        {
            return None
        }
        _ => {}
    }
    if !filter.high_volume && ev.is_high_volume() {
        return None;
    }
    if let Some(set) = &filter.tasks {
        if let Event::Progress(rows) = ev {
            let rows: Vec<_> = rows
                .iter()
                .filter(|p| set.contains(p.task_id.as_str()))
                .collect();
            if rows.is_empty() {
                return None;
            }
            return Some((ev.type_name(), serde_json::to_value(rows).ok()?));
        }
        if let Some(id) = ev.task_id() {
            if !set.contains(id.as_str()) {
                return None;
            }
        }
    }
    let data = match ev {
        Event::TaskAdded(t) | Event::TaskUpdated(t) => serde_json::to_value(TaskRow::from(&**t)).ok()?,
        Event::PairingStarted {
            expires_at, url, ..
        } if remote => {
            // The code itself never leaves this machine over the network.
            json!({ "code": null, "expires_at": expires_at, "url": url })
        }
        _ => match serde_json::to_value(ev).ok()? {
            Value::Object(mut m) => m.remove("data").unwrap_or(Value::Null),
            _ => Value::Null,
        },
    };
    Some((ev.type_name(), data))
}

/// Does this event end the session (the caller's own device was revoked/removed)?
fn revokes(ev: &Event, caller: &Caller) -> bool {
    if caller.trusted {
        return false;
    }
    match ev {
        Event::DeviceRemoved { device_id } => *device_id == caller.device.id,
        Event::DeviceUpdated(d) => d.id == caller.device.id && d.revoked,
        _ => false,
    }
}

fn text(v: Value) -> Message {
    Message::Text(Utf8Bytes::from(v.to_string()))
}

async fn run(socket: WebSocket, st: AppState, caller: Caller, mut filter: Filter) {
    let (mut tx, mut rx) = socket.split();
    let mut events = st.engine.subscribe();
    let mut seq: u64 = 0;

    macro_rules! send {
        ($msg:expr) => {
            match tokio::time::timeout(SEND_TIMEOUT, tx.send($msg)).await {
                Ok(Ok(())) => {}
                _ => return,
            }
        };
    }

    let hello = json!({
        "type": "hello",
        "data": { "version": st.engine.info().version, "seq": seq, "resync": true },
        "seq": seq,
    });
    send!(text(hello));

    let mut last_seen = Instant::now();
    let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + PING_EVERY, PING_EVERY);

    loop {
        tokio::select! {
            _ = st.shutdown.cancelled() => {
                let _ = tx.send(Message::Close(Some(CloseFrame {
                    code: axum::extract::ws::close_code::AWAY,
                    reason: Utf8Bytes::from_static("server shutting down"),
                }))).await;
                return;
            }
            ev = events.recv() => match ev {
                Ok(ev) => {
                    if revokes(&ev, &caller) {
                        let _ = tx.send(Message::Close(Some(CloseFrame {
                            code: axum::extract::ws::close_code::POLICY,
                            reason: Utf8Bytes::from_static("device revoked"),
                        }))).await;
                        return;
                    }
                    if let Some((ty, data)) = project(&ev, &caller, &filter) {
                        seq += 1;
                        send!(text(json!({ "type": ty, "data": data, "seq": seq })));
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    seq += 1;
                    send!(text(json!({ "type": "lagged", "data": { "missed": n }, "seq": seq })));
                }
                Err(RecvError::Closed) => {
                    let _ = tx.send(Message::Close(None)).await;
                    return;
                }
            },
            msg = rx.next() => {
                let Some(Ok(msg)) = msg else { return };
                last_seen = Instant::now();
                match msg {
                    Message::Text(t) => {
                        let Ok(v) = serde_json::from_str::<Value>(t.as_str()) else { continue };
                        match v.get("type").and_then(Value::as_str) {
                            Some("ping") => send!(text(json!({ "type": "pong" }))),
                            Some("subscribe") => {
                                let ids: Vec<String> = v
                                    .get("tasks")
                                    .and_then(Value::as_array)
                                    .map(|a| {
                                        a.iter()
                                            .filter_map(Value::as_str)
                                            .map(str::to_owned)
                                            .collect()
                                    })
                                    .unwrap_or_default();
                                filter.tasks = parse_tasks(ids);
                                if let Some(hv) = v.get("high_volume").and_then(Value::as_bool) {
                                    filter.high_volume = hv;
                                }
                            }
                            _ => {}
                        }
                    }
                    Message::Close(_) => return,
                    // Ping is answered automatically; Pong/Binary only count as activity.
                    _ => {}
                }
            }
            _ = ping.tick() => {
                if last_seen.elapsed() >= IDLE_CLOSE {
                    let _ = tx.send(Message::Close(Some(CloseFrame {
                        code: axum::extract::ws::close_code::AWAY,
                        reason: Utf8Bytes::from_static("idle timeout"),
                    }))).await;
                    return;
                }
                send!(Message::Ping(bytes::Bytes::new()));
            }
        }
    }
}
