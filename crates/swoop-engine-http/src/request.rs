//! Request construction: the per-task header set (user headers, referer, cookies, credential
//! headers, Basic auth) and the client profile. Built once per run and applied to every
//! probe and segment request so all connections look identical to the server.

use http::header::{
    HeaderMap, HeaderName, HeaderValue, ACCEPT, ACCEPT_ENCODING, COOKIE, REFERER, USER_AGENT,
};
use swoop_domain::settings::Settings;
use swoop_domain::{ErrorKind, Task, TaskError};
use swoop_runtime::engine::TransferSecrets;
use swoop_runtime::net::{validate_headers, ClientProfile};
use url::Url;

/// Everything that must be attached to every request of a task.
#[derive(Clone, Debug)]
pub struct RequestTemplate {
    headers: HeaderMap,
    basic: Option<(String, Option<String>)>,
}

impl RequestTemplate {
    /// Build from task options and resolved secrets. User headers are validated (no hop-by-hop
    /// or engine-managed names, no CR/LF); credential headers get the same treatment because a
    /// credential store entry is still data typed by a person.
    pub fn build(task: &Task, secrets: &TransferSecrets) -> Result<Self, TaskError> {
        let mut headers = HeaderMap::new();
        // Downloads must receive raw bytes: content-coding breaks Range and Content-Length.
        headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
        headers.insert(ACCEPT, HeaderValue::from_static("*/*"));

        let user: Vec<(&str, &str)> = task
            .options
            .headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        for (name, value) in validate_headers(user)? {
            insert(&mut headers, &name, &value)?;
        }
        let extra: Vec<(&str, &str)> = secrets
            .headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        for (name, value) in validate_headers(extra)? {
            insert(&mut headers, &name, &value)?;
        }
        if let Some(ua) = task.options.user_agent.as_deref() {
            insert(&mut headers, USER_AGENT.as_str(), ua)?;
        }
        if let Some(referer) = task.options.referer.as_deref() {
            insert(&mut headers, REFERER.as_str(), referer)?;
        }
        if let Some(cookies) = task.options.cookies.as_deref() {
            insert(&mut headers, COOKIE.as_str(), cookies)?;
        }
        let basic = secrets
            .username
            .clone()
            .map(|u| (u, secrets.password.clone()));
        Ok(Self { headers, basic })
    }

    /// Apply the template to a request builder.
    pub fn apply(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let rb = rb.headers(self.headers.clone());
        match &self.basic {
            Some((user, pass)) => rb.basic_auth(user, pass.as_deref()),
            None => rb,
        }
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }
}

fn insert(headers: &mut HeaderMap, name: &str, value: &str) -> Result<(), TaskError> {
    let n = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
        TaskError::new(
            ErrorKind::InvalidUrl,
            format!("invalid header name {name:?}"),
        )
    })?;
    let v = HeaderValue::from_str(value).map_err(|_| {
        TaskError::new(
            ErrorKind::InvalidUrl,
            format!("invalid value for header {name:?}"),
        )
    })?;
    headers.insert(n, v);
    Ok(())
}

/// The client profile for a task: HTTP/1.1-only unless HTTP/2 is explicitly allowed (each
/// segment must own a TCP connection for range parallelism to help), proxy from secrets,
/// TLS exception only when the host is on the user's exception list.
pub fn client_profile(
    task: &Task,
    settings: &Settings,
    secrets: &TransferSecrets,
) -> ClientProfile {
    let host = task
        .source
        .primary_url()
        .and_then(|u| Url::parse(u).ok())
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()));
    let tls_exception_host = host.filter(|h| {
        settings
            .network
            .tls_exceptions
            .iter()
            .any(|e| e.eq_ignore_ascii_case(h))
    });
    ClientProfile {
        http1_only: !task
            .options
            .allow_http2
            .unwrap_or(settings.network.allow_http2_for_segments),
        proxy_url: secrets.proxy_url.clone(),
        direct: task.options.direct_connection,
        tls_exception_host,
        cookies: true,
        user_agent: task.options.user_agent.clone(),
    }
}

/// Reject anything that is not `http`/`https` before a socket is touched.
pub fn parse_http_url(raw: &str) -> Result<Url, TaskError> {
    let url = Url::parse(raw.trim()).map_err(|e| {
        TaskError::new(ErrorKind::InvalidUrl, format!("invalid URL: {e}"))
            .with_source(swoop_runtime::redact::redact(raw))
    })?;
    match url.scheme() {
        "http" | "https" => {}
        other => {
            return Err(TaskError::new(
                ErrorKind::UnsupportedScheme,
                format!("unsupported scheme {other:?}"),
            )
            .with_source(swoop_runtime::redact::redact(raw)))
        }
    }
    if url.host_str().is_none() {
        return Err(TaskError::new(ErrorKind::InvalidUrl, "URL has no host")
            .with_source(swoop_runtime::redact::redact(raw)));
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use swoop_domain::{QueueId, Source, TaskKind};

    fn task() -> Task {
        Task::new(
            TaskKind::Http,
            Source::Urls {
                urls: vec!["https://example.com/a.zip".into()],
            },
            "a.zip",
            PathBuf::from("/tmp"),
            QueueId::default_queue(),
        )
    }

    #[test]
    fn builds_headers_and_rejects_forbidden() {
        let mut t = task();
        t.options.headers.insert("X-Api-Key".into(), "abc".into());
        t.options.referer = Some("https://example.com/page".into());
        t.options.cookies = Some("a=1; b=2".into());
        let secrets = TransferSecrets {
            username: Some("u".into()),
            password: Some("p".into()),
            headers: vec![("Authorization".into(), "Bearer t".into())],
            ..Default::default()
        };
        let tpl = RequestTemplate::build(&t, &secrets).unwrap();
        assert_eq!(tpl.headers().get("x-api-key").unwrap(), "abc");
        assert_eq!(
            tpl.headers().get("referer").unwrap(),
            "https://example.com/page"
        );
        assert_eq!(tpl.headers().get("cookie").unwrap(), "a=1; b=2");
        assert_eq!(tpl.headers().get("accept-encoding").unwrap(), "identity");
        t.options.headers.insert("Range".into(), "bytes=0-1".into());
        assert!(RequestTemplate::build(&t, &secrets).is_err());
    }

    #[test]
    fn scheme_rejection() {
        assert_eq!(
            parse_http_url("file:///etc/passwd").unwrap_err().kind,
            ErrorKind::UnsupportedScheme
        );
        assert_eq!(
            parse_http_url("ftp://h/x").unwrap_err().kind,
            ErrorKind::UnsupportedScheme
        );
        assert_eq!(
            parse_http_url("nope").unwrap_err().kind,
            ErrorKind::InvalidUrl
        );
        assert!(parse_http_url("https://h/x").is_ok());
    }

    #[test]
    fn profile_follows_settings() {
        let t = task();
        let mut s = Settings::default();
        let p = client_profile(&t, &s, &TransferSecrets::default());
        assert!(p.http1_only);
        assert!(p.tls_exception_host.is_none());
        s.network.tls_exceptions.push("EXAMPLE.com".into());
        let p = client_profile(&t, &s, &TransferSecrets::default());
        assert_eq!(p.tls_exception_host.as_deref(), Some("example.com"));
    }
}
