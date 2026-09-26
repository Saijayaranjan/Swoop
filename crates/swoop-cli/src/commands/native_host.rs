//! `native-host`: the Chrome/Firefox native-messaging relay, and `--install-manifest`.
//!
//! Framing: 4-byte little-endian length prefix + UTF-8 JSON, max 1 MiB per message, on
//! stdin/stdout. The host is a stateless relay to the local API: it never uses `--url`/`--token`
//! or the remote listener, only the local Unix socket and the local token file.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use http::Method;
use serde_json::Value;
use tokio::sync::{mpsc, Mutex};

use crate::cli::{BrowserArg, NativeHostArgs};
use crate::error::{CliError, CliResult};
use crate::resolve::build_client;
use crate::transport::{Client, Endpoint};

pub const MAX_MESSAGE_BYTES: u32 = 1024 * 1024;
const HOST_NAME: &str = "app.swoop.bridge";
const BUNDLE_ID: &str = "app.swoop.desktop";

/// The Chromium extension id every host manifest allows. Chrome derives it from the public `key`
/// in extensions/browser/manifests/{chrome,edge}.json, so an unpacked build has this id in every
/// browser and on every machine (the macOS app's `NativeMessagingInstaller.chromiumExtensionId`
/// must match).
pub const CHROMIUM_EXTENSION_ID: &str = "hbfgocpejejjhpigpanikchicoplcfjb";
/// Ids assigned by an extension store, if the published build ever carries a different key.
const CHROMIUM_STORE_EXTENSION_IDS: &[&str] = &[];
/// Firefox's fixed gecko id, from extensions/browser/manifests/firefox.json.
pub const FIREFOX_EXTENSION_ID: &str = "swoop@swoop.app";

/// The only `/api/v1/` groups the extension uses (adding/controlling downloads and media
/// detection). Everything else — settings, devices, automations, rules, queues, updates,
/// import/export, archives, plugins — is refused: the relay authenticates with the local
/// admin token, so an allowlist keeps a compromised extension from reaching the rest.
const ALLOWED_GROUPS: &[&str] = &["tasks", "media"];

// ---------------------------------------------------------------------------------------------
// Framing (sync; run from a blocking thread since stdin/stdout are blocking).
// ---------------------------------------------------------------------------------------------

/// Read one framed message. Returns `Ok(None)` on a clean EOF before any bytes of the next
/// frame arrive.
pub fn read_message(r: &mut impl Read) -> std::io::Result<Option<Value>> {
    let mut len_buf = [0u8; 4];
    if !read_exact_or_eof(r, &mut len_buf)? {
        return Ok(None);
    }
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_MESSAGE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("native message too large: {len} bytes (max {MAX_MESSAGE_BYTES})"),
        ));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf)?;
    serde_json::from_slice(&buf)
        .map(Some)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Like `read_exact`, but a zero-byte read on the first byte is treated as EOF (`Ok(false)`)
/// rather than an error.
fn read_exact_or_eof(r: &mut impl Read, buf: &mut [u8]) -> std::io::Result<bool> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) if filled == 0 => return Ok(false),
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "truncated native message length prefix",
                ))
            }
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(true)
}

/// Write one framed message.
pub fn write_message(w: &mut impl Write, value: &Value) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() as u64 > MAX_MESSAGE_BYTES as u64 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "outgoing native message exceeds 1 MiB",
        ));
    }
    w.write_all(&(bytes.len() as u32).to_le_bytes())?;
    w.write_all(&bytes)?;
    w.flush()
}

fn path_allowed(path: &str) -> bool {
    // Reject anything a server or proxy might normalise into a different route.
    if path.len() > 2048
        || path.contains("..")
        || path.contains("//")
        || path.contains('\\')
        || path.contains('%')
        || path.contains('#')
        || path.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return false;
    }
    let Some(rest) = path
        .split('?')
        .next()
        .unwrap_or("")
        .strip_prefix("/api/v1/")
    else {
        return false;
    };
    let first = rest.split('/').next().unwrap_or("");
    ALLOWED_GROUPS.contains(&first)
}

fn method_allowed(m: &str) -> bool {
    matches!(m, "GET" | "POST" | "PUT" | "PATCH" | "DELETE")
}

// ---------------------------------------------------------------------------------------------
// The relay
// ---------------------------------------------------------------------------------------------

pub async fn run(cli: &crate::cli::Cli, args: NativeHostArgs) -> CliResult<()> {
    if let Some(browser) = args.install_manifest {
        let exe = std::env::current_exe()?;
        let browsers: Vec<BrowserArg> = if browser == BrowserArg::All {
            ALL_BROWSERS
                .iter()
                .copied()
                .filter(|b| browser_dirs(*b).is_some_and(|(marker, _)| marker.exists()))
                .collect()
        } else {
            vec![browser]
        };
        if browsers.is_empty() {
            eprintln!("no supported browsers found");
        }
        for b in browsers {
            let path = install_manifest(b, args.extension_id.as_deref(), &exe)?;
            eprintln!(
                "{}: native messaging host manifest at {}",
                browser_name(b),
                path.display()
            );
        }
        return Ok(());
    }

    // The host always talks to the local socket, never a remote/--url server.
    let local_cli = crate::cli::Cli {
        json: cli.json,
        data_dir: cli.data_dir.clone(),
        socket: cli.socket.clone(),
        url: None,
        token: None,
        log_level: cli.log_level.clone(),
        headless: false,
        command: None,
    };
    let client = build_client(&local_cli)?;

    let (in_tx, mut in_rx) = mpsc::unbounded_channel::<Value>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut lock = stdin.lock();
        loop {
            match read_message(&mut lock) {
                Ok(Some(v)) => {
                    if in_tx.send(v).is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }
    });

    let write_lock: std::sync::Arc<Mutex<()>> = std::sync::Arc::new(Mutex::new(()));
    let subscription: std::sync::Arc<Mutex<Option<tokio::task::JoinHandle<()>>>> =
        std::sync::Arc::new(Mutex::new(None));

    while let Some(msg) = in_rx.recv().await {
        let client = client.clone();
        let write_lock = write_lock.clone();
        let subscription = subscription.clone();
        tokio::spawn(async move {
            handle_message(&client, msg, write_lock, subscription).await;
        });
    }

    Ok(())
}

async fn send_frame(write_lock: &std::sync::Arc<Mutex<()>>, value: &Value) {
    let _guard = write_lock.lock().await;
    let mut stdout = std::io::stdout();
    let _ = write_message(&mut stdout, value);
}

async fn handle_message(
    client: &Client,
    msg: Value,
    write_lock: std::sync::Arc<Mutex<()>>,
    subscription: std::sync::Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
) {
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    let ty = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match ty {
        "ping" => {
            let running = client.get("/healthz").await.is_ok();
            send_frame(
                &write_lock,
                &serde_json::json!({ "type": "pong", "version": env!("CARGO_PKG_VERSION"), "running": running }),
            )
            .await;
        }
        "request" => {
            let method_str = msg.get("method").and_then(|v| v.as_str()).unwrap_or("GET");
            let path = msg.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let body = msg.get("body").cloned();
            if !path_allowed(path) {
                send_frame(
                    &write_lock,
                    &serde_json::json!({ "id": id, "ok": false, "error": { "type": "permission_denied", "message": "path not relayed to extensions" } }),
                )
                .await;
                return;
            }
            let method = match Method::from_bytes(method_str.as_bytes()) {
                Ok(m) if method_allowed(method_str) => m,
                _ => {
                    send_frame(
                        &write_lock,
                        &serde_json::json!({ "id": id, "ok": false, "error": { "type": "validation", "message": "bad method" } }),
                    )
                    .await;
                    return;
                }
            };
            match client.request(method, path, body).await {
                Ok(value) => {
                    send_frame(
                        &write_lock,
                        &serde_json::json!({ "id": id, "ok": true, "status": 200, "body": value }),
                    )
                    .await;
                }
                Err(CliError::NotRunning(message)) => {
                    send_frame(
                        &write_lock,
                        &serde_json::json!({ "id": id, "ok": false, "error": { "type": "unavailable", "message": message } }),
                    )
                    .await;
                }
                Err(CliError::Api {
                    status,
                    kind,
                    message,
                }) => {
                    send_frame(
                        &write_lock,
                        &serde_json::json!({ "id": id, "ok": false, "status": status, "error": { "type": kind, "message": message } }),
                    )
                    .await;
                }
                Err(e) => {
                    send_frame(
                        &write_lock,
                        &serde_json::json!({ "id": id, "ok": false, "error": { "type": "internal", "message": e.to_string() } }),
                    )
                    .await;
                }
            }
        }
        "subscribe" => {
            let tasks: Vec<String> = msg
                .get("tasks")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            let events_filter: Option<Vec<String>> =
                msg.get("events").and_then(|v| v.as_array()).map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                });
            let client = client.clone();
            let write_lock_task = write_lock.clone();
            let handle = tokio::spawn(async move {
                let _ = run_subscription(client, tasks, events_filter, write_lock_task).await;
            });
            let mut slot = subscription.lock().await;
            if let Some(old) = slot.take() {
                old.abort();
            }
            *slot = Some(handle);
            send_frame(&write_lock, &serde_json::json!({ "id": id, "ok": true })).await;
        }
        "unsubscribe" => {
            let mut slot = subscription.lock().await;
            if let Some(old) = slot.take() {
                old.abort();
            }
            send_frame(&write_lock, &serde_json::json!({ "id": id, "ok": true })).await;
        }
        "launch" => {
            let ok = launch_app();
            if ok {
                send_frame(&write_lock, &serde_json::json!({ "id": id, "ok": true })).await;
            } else {
                send_frame(
                    &write_lock,
                    &serde_json::json!({ "id": id, "ok": false, "error": { "type": "unavailable", "message": "launch is only supported on macOS" } }),
                )
                .await;
            }
        }
        other => {
            send_frame(
                &write_lock,
                &serde_json::json!({ "id": id, "ok": false, "error": { "type": "validation", "message": format!("unknown message type `{other}`") } }),
            )
            .await;
        }
    }
}

async fn run_subscription(
    client: Client,
    tasks: Vec<String>,
    events_filter: Option<Vec<String>>,
    write_lock: std::sync::Arc<Mutex<()>>,
) -> CliResult<()> {
    use futures::StreamExt;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::Message;

    let Endpoint::Socket { path, token } = client.endpoint() else {
        return Err(CliError::Other(anyhow::anyhow!(
            "native-host subscriptions require the local socket"
        )));
    };
    let mut qs: Vec<(&str, Option<String>)> = vec![("token", Some(token.clone()))];
    if !tasks.is_empty() {
        qs.push(("tasks", Some(tasks.join(","))));
    }
    let query = crate::transport::build_query(&qs);
    let stream = tokio::net::UnixStream::connect(path)
        .await
        .map_err(|_| CliError::not_running())?;
    let request = format!("ws://localhost/api/v1/events{query}")
        .into_client_request()
        .map_err(|e| CliError::Other(anyhow::anyhow!(e)))?;
    let (mut ws, _) = tokio_tungstenite::client_async(request, stream)
        .await
        .map_err(|e| CliError::Other(anyhow::anyhow!("websocket connect failed: {e}")))?;

    while let Some(msg) = ws.next().await {
        let Ok(Message::Text(text)) = msg else {
            continue;
        };
        let Ok(frame) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let ty = frame.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(filter) = &events_filter {
            if !filter.iter().any(|f| f == ty) {
                continue;
            }
        }
        let event = serde_json::json!({ "type": ty, "data": frame.get("data").cloned().unwrap_or(Value::Null) });
        send_frame(
            &write_lock,
            &serde_json::json!({ "type": "event", "event": event }),
        )
        .await;
    }
    Ok(())
}

fn launch_app() -> bool {
    if cfg!(target_os = "macos") {
        std::process::Command::new("open")
            .args(["-b", BUNDLE_ID])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    } else {
        false
    }
}

// ---------------------------------------------------------------------------------------------
// `--install-manifest`
// ---------------------------------------------------------------------------------------------

const ALL_BROWSERS: &[BrowserArg] = &[
    BrowserArg::Chrome,
    BrowserArg::ChromeBeta,
    BrowserArg::ChromeCanary,
    BrowserArg::Chromium,
    BrowserArg::Brave,
    BrowserArg::Edge,
    BrowserArg::Vivaldi,
    BrowserArg::Arc,
    BrowserArg::Opera,
    BrowserArg::Firefox,
];

fn browser_name(browser: BrowserArg) -> String {
    use clap::ValueEnum;
    browser
        .to_possible_value()
        .map(|v| v.get_name().to_owned())
        .unwrap_or_default()
}

/// `(a file whose presence means the browser has been used, its native-messaging-hosts folder)`,
/// or `None` when the browser has no such location on this platform. The marker is a file inside
/// the profile folder rather than the folder itself, because other tools create bare
/// `NativeMessagingHosts` folders for browsers that aren't installed.
fn browser_dirs(browser: BrowserArg) -> Option<(PathBuf, PathBuf)> {
    use BrowserArg::*;
    let home = directories::BaseDirs::new()?.home_dir().to_path_buf();
    if cfg!(target_os = "macos") {
        let support = home.join("Library/Application Support");
        let (profile, hosts) = match browser {
            All => return None,
            Chrome => ("Google/Chrome", "Google/Chrome"),
            ChromeBeta => ("Google/Chrome Beta", "Google/Chrome Beta"),
            ChromeCanary => ("Google/Chrome Canary", "Google/Chrome Canary"),
            Chromium => ("Chromium", "Chromium"),
            Brave => ("BraveSoftware/Brave-Browser", "BraveSoftware/Brave-Browser"),
            Edge => ("Microsoft Edge", "Microsoft Edge"),
            Vivaldi => ("Vivaldi", "Vivaldi"),
            Arc => ("Arc/User Data", "Arc/User Data"),
            // Opera reads Chrome's host folder.
            Opera => ("com.operasoftware.Opera", "Google/Chrome"),
            Firefox => ("Mozilla", "Mozilla"),
        };
        let marker = if browser == Firefox {
            support.join("Firefox/profiles.ini")
        } else {
            support.join(profile).join("Local State")
        };
        Some((marker, support.join(hosts).join("NativeMessagingHosts")))
    } else if cfg!(target_os = "linux") {
        let (profile, hosts) = match browser {
            All | Arc => return None,
            Chrome => (".config/google-chrome", ".config/google-chrome"),
            ChromeBeta => (".config/google-chrome-beta", ".config/google-chrome-beta"),
            ChromeCanary => (
                ".config/google-chrome-unstable",
                ".config/google-chrome-unstable",
            ),
            Chromium => (".config/chromium", ".config/chromium"),
            Brave => (
                ".config/BraveSoftware/Brave-Browser",
                ".config/BraveSoftware/Brave-Browser",
            ),
            Edge => (".config/microsoft-edge", ".config/microsoft-edge"),
            Vivaldi => (".config/vivaldi", ".config/vivaldi"),
            Opera => (".config/opera", ".config/google-chrome"),
            Firefox => {
                return Some((
                    home.join(".mozilla/firefox/profiles.ini"),
                    home.join(".mozilla/native-messaging-hosts"),
                ))
            }
        };
        Some((
            home.join(profile).join("Local State"),
            home.join(hosts).join("NativeMessagingHosts"),
        ))
    } else {
        None
    }
}

/// Writes the host manifest for `browser`, pointing at `exe`, and returns its path. The file is
/// left untouched when it already has these contents.
fn install_manifest(browser: BrowserArg, extra_id: Option<&str>, exe: &Path) -> CliResult<PathBuf> {
    let (_, hosts) = browser_dirs(browser).ok_or_else(|| {
        CliError::Usage(format!(
            "no native messaging host location for {} on this platform",
            browser_name(browser)
        ))
    })?;
    let manifest = build_manifest_json(browser, extra_id, exe);
    let contents = serde_json::to_string_pretty(&manifest)? + "\n";
    let path = hosts.join(format!("{HOST_NAME}.json"));
    if std::fs::read_to_string(&path).ok().as_deref() != Some(contents.as_str()) {
        std::fs::create_dir_all(&hosts)?;
        std::fs::write(&path, contents)?;
    }
    Ok(path)
}

/// The `app.swoop.bridge` manifest: Swoop's own extension ids, plus `extra_id` if given.
fn build_manifest_json(browser: BrowserArg, extra_id: Option<&str>, exe: &Path) -> Value {
    let mut m = serde_json::json!({
        "name": HOST_NAME,
        "description": "Swoop native messaging host",
        "path": exe.to_string_lossy(),
        "type": "stdio",
    });
    if browser == BrowserArg::Firefox {
        let ids: Vec<&str> = std::iter::once(FIREFOX_EXTENSION_ID)
            .chain(extra_id.filter(|id| *id != FIREFOX_EXTENSION_ID))
            .collect();
        m["allowed_extensions"] = serde_json::json!(ids);
    } else {
        let mut ids = vec![CHROMIUM_EXTENSION_ID];
        ids.extend_from_slice(CHROMIUM_STORE_EXTENSION_IDS);
        ids.extend(extra_id.filter(|id| !ids.contains(id)));
        let origins: Vec<String> = ids
            .iter()
            .map(|id| format!("chrome-extension://{id}/"))
            .collect();
        m["allowed_origins"] = serde_json::json!(origins);
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn round_trips_a_message() {
        let mut buf = Vec::new();
        let value = serde_json::json!({ "id": 1, "type": "ping" });
        write_message(&mut buf, &value).unwrap();
        let mut cursor = Cursor::new(buf);
        let read_back = read_message(&mut cursor).unwrap().unwrap();
        assert_eq!(read_back, value);
    }

    #[test]
    fn clean_eof_before_any_bytes_is_none() {
        let mut cursor = Cursor::new(Vec::<u8>::new());
        assert!(read_message(&mut cursor).unwrap().is_none());
    }

    #[test]
    fn truncated_length_prefix_errors() {
        let mut cursor = Cursor::new(vec![1, 2]);
        assert!(read_message(&mut cursor).is_err());
    }

    #[test]
    fn oversized_message_is_refused() {
        let mut buf = Vec::new();
        let len = MAX_MESSAGE_BYTES + 1;
        buf.extend_from_slice(&len.to_le_bytes());
        let mut cursor = Cursor::new(buf);
        assert!(read_message(&mut cursor).is_err());
    }

    #[test]
    fn multiple_frames_read_in_sequence() {
        let mut buf = Vec::new();
        write_message(&mut buf, &serde_json::json!({"n": 1})).unwrap();
        write_message(&mut buf, &serde_json::json!({"n": 2})).unwrap();
        let mut cursor = Cursor::new(buf);
        let a = read_message(&mut cursor).unwrap().unwrap();
        let b = read_message(&mut cursor).unwrap().unwrap();
        assert_eq!(a["n"], 1);
        assert_eq!(b["n"], 2);
        assert!(read_message(&mut cursor).unwrap().is_none());
    }

    #[test]
    fn refuses_forbidden_paths_but_allows_task_routes() {
        assert!(path_allowed("/api/v1/tasks"));
        assert!(path_allowed("/api/v1/tasks/abc/pause"));
        assert!(!path_allowed("/api/v1/settings"));
        assert!(!path_allowed("/api/v1/devices"));
        assert!(!path_allowed("/api/v1/automations"));
        assert!(!path_allowed("/healthz"));
        assert!(!path_allowed("/api/v2/tasks"));
        assert!(!path_allowed("/api/v1/rules"));
        assert!(!path_allowed("/api/v1/tasks/../settings"));
        assert!(!path_allowed("/api/v1/tasks/%2e%2e/settings"));
        assert!(path_allowed("/api/v1/tasks/rows?smart=active"));
        assert!(path_allowed("/api/v1/media/detect"));
    }

    #[test]
    fn manifest_allows_swoops_own_extension_ids() {
        let exe = Path::new("/Applications/Swoop.app/Contents/Helpers/swoop");
        let chrome = build_manifest_json(BrowserArg::Brave, None, exe);
        assert_eq!(chrome["name"], HOST_NAME);
        assert_eq!(chrome["path"], exe.to_str().unwrap());
        assert_eq!(
            chrome["allowed_origins"],
            serde_json::json!([format!("chrome-extension://{CHROMIUM_EXTENSION_ID}/")])
        );
        assert!(chrome.get("allowed_extensions").is_none());

        let firefox = build_manifest_json(BrowserArg::Firefox, None, exe);
        assert_eq!(
            firefox["allowed_extensions"],
            serde_json::json!(["swoop@swoop.app"])
        );
        assert!(firefox.get("allowed_origins").is_none());
    }
}
