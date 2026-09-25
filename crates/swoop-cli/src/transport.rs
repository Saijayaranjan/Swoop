//! HTTP transport to the running engine: either the local Unix socket (hyper directly — reqwest
//! cannot dial a Unix socket) or a remote/local HTTP(S) server (plain reqwest).

use std::path::{Path, PathBuf};

use bytes::Bytes;
use http::{Method, Request, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use serde_json::Value;
use tokio::net::UnixStream;

use crate::error::CliError;

/// Where the CLI sends requests.
#[derive(Clone)]
pub enum Endpoint {
    Socket { path: PathBuf, token: String },
    Remote { base: String, token: Option<String> },
}

#[derive(Clone)]
pub struct Client {
    endpoint: Endpoint,
}

/// The `{"error": {"type": "...", "message": "..."}}` envelope every API error uses.
#[derive(serde::Deserialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(serde::Deserialize)]
struct ErrorBody {
    #[serde(rename = "type")]
    kind: String,
    message: String,
}

impl Client {
    pub fn new(endpoint: Endpoint) -> Self {
        Self { endpoint }
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Send a request and return the decoded JSON body, or a [`CliError`] mapped from the
    /// error envelope / connection failure.
    pub async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, CliError> {
        let (status, value) = match &self.endpoint {
            Endpoint::Socket { path: sock, token } => {
                socket_request(sock, token, method, path, body).await?
            }
            Endpoint::Remote { base, token } => {
                remote_request(base, token.as_deref(), method, path, body).await?
            }
        };
        if status.is_success() {
            Ok(value)
        } else if let Ok(env) = serde_json::from_value::<ErrorEnvelope>(value.clone()) {
            Err(CliError::Api {
                status: status.as_u16(),
                kind: env.error.kind,
                message: env.error.message,
            })
        } else {
            Err(CliError::Api {
                status: status.as_u16(),
                kind: "internal".to_owned(),
                message: format!("HTTP {status}"),
            })
        }
    }

    pub async fn get(&self, path: &str) -> Result<Value, CliError> {
        self.request(Method::GET, path, None).await
    }

    pub async fn post(&self, path: &str, body: Value) -> Result<Value, CliError> {
        self.request(Method::POST, path, Some(body)).await
    }

    pub async fn post_empty(&self, path: &str) -> Result<Value, CliError> {
        self.request(Method::POST, path, None).await
    }

    /// Not currently used by any command (no `swoop` command needs `PUT`), kept for
    /// completeness of the transport's HTTP verb surface.
    #[allow(dead_code)]
    pub async fn put(&self, path: &str, body: Value) -> Result<Value, CliError> {
        self.request(Method::PUT, path, Some(body)).await
    }

    /// Not currently used by any command (no `swoop` command needs `PATCH`), kept for
    /// completeness of the transport's HTTP verb surface.
    #[allow(dead_code)]
    pub async fn patch(&self, path: &str, body: Value) -> Result<Value, CliError> {
        self.request(Method::PATCH, path, Some(body)).await
    }

    pub async fn delete(&self, path: &str) -> Result<Value, CliError> {
        self.request(Method::DELETE, path, None).await
    }

    /// GET a path and return the raw response body as text (for `diagnostics.txt`).
    pub async fn get_text(&self, path: &str) -> Result<String, CliError> {
        match &self.endpoint {
            Endpoint::Socket { path: sock, token } => {
                let (status, bytes) =
                    socket_request_raw(sock, token, Method::GET, path, None).await?;
                if !status.is_success() {
                    return Err(CliError::Api {
                        status: status.as_u16(),
                        kind: "internal".to_owned(),
                        message: format!("HTTP {status}"),
                    });
                }
                Ok(String::from_utf8_lossy(&bytes).into_owned())
            }
            Endpoint::Remote { base, token } => {
                let url = format!("{}{}", base.trim_end_matches('/'), path);
                let http = reqwest::Client::new();
                let mut req = http.get(&url);
                if let Some(t) = token {
                    req = req.bearer_auth(t);
                }
                let resp = req
                    .send()
                    .await
                    .map_err(|e| CliError::NotRunning(format!("cannot reach {url}: {e}")))?;
                if !resp.status().is_success() {
                    return Err(CliError::Api {
                        status: resp.status().as_u16(),
                        kind: "internal".to_owned(),
                        message: format!("HTTP {}", resp.status()),
                    });
                }
                resp.text()
                    .await
                    .map_err(|e| CliError::Other(anyhow::anyhow!(e)))
            }
        }
    }
}

async fn socket_request(
    socket_path: &Path,
    token: &str,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<(StatusCode, Value), CliError> {
    let (status, bytes) = socket_request_raw(socket_path, token, method, path, body).await?;
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)?
    };
    Ok((status, value))
}

async fn socket_request_raw(
    socket_path: &Path,
    token: &str,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<(StatusCode, Bytes), CliError> {
    let stream = UnixStream::connect(socket_path)
        .await
        .map_err(|_| CliError::not_running())?;
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
        .await
        .map_err(|e| CliError::Other(anyhow::anyhow!("handshake failed: {e}")))?;
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let body_bytes: Vec<u8> = match &body {
        Some(v) => serde_json::to_vec(v)?,
        None => Vec::new(),
    };
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "localhost")
        .header("authorization", format!("Bearer {token}"));
    if !body_bytes.is_empty() {
        builder = builder.header("content-type", "application/json");
    }
    let req = builder
        .body(Full::new(Bytes::from(body_bytes)))
        .map_err(|e| CliError::Other(anyhow::anyhow!(e)))?;

    let resp = sender
        .send_request(req)
        .await
        .map_err(|e| CliError::Other(anyhow::anyhow!("request failed: {e}")))?;
    let status = resp.status();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .map_err(|e| CliError::Other(anyhow::anyhow!("reading response failed: {e}")))?
        .to_bytes();
    Ok((status, bytes))
}

async fn remote_request(
    base: &str,
    token: Option<&str>,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<(StatusCode, Value), CliError> {
    let url = format!("{}{}", base.trim_end_matches('/'), path);
    let http = reqwest::Client::new();
    let mut req = http.request(method, &url);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    if let Some(b) = &body {
        req = req.json(b);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| CliError::NotRunning(format!("cannot reach {url}: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| CliError::Other(anyhow::anyhow!(e)))?;
    let value = if text.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text)?
    };
    Ok((status, value))
}

/// Build a query string from `key=value` pairs (values already stringified), percent-encoding
/// keys and values. Omits pairs whose value is `None`.
pub fn build_query(pairs: &[(&str, Option<String>)]) -> String {
    use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
    const FRAGMENT: &AsciiSet = &CONTROLS
        .add(b' ')
        .add(b'"')
        .add(b'#')
        .add(b'<')
        .add(b'>')
        .add(b'&')
        .add(b'=')
        .add(b'?')
        .add(b'+');
    let mut out = String::new();
    for (k, v) in pairs {
        let Some(v) = v else { continue };
        if v.is_empty() {
            continue;
        }
        if out.is_empty() {
            out.push('?');
        } else {
            out.push('&');
        }
        out.push_str(&utf8_percent_encode(k, FRAGMENT).to_string());
        out.push('=');
        out.push_str(&utf8_percent_encode(v, FRAGMENT).to_string());
    }
    out
}
