//! The crawl loop: bounded BFS with per-host politeness, robots.txt, scope and page limits.

use crate::classify::{accept_file, default_extensions, kind_for_mime, looks_like_page};
use crate::extract::extract;
use crate::robots::Robots;
use crate::types::{GrabberFile, GrabberOptions, GrabberSession};
use crate::urlnorm::{normalize, Scope};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use swoop_domain::{ErrorKind, Millis, TaskError};
use swoop_runtime::net::ClientFactory;
use tokio::sync::{watch, Semaphore};
use tokio_util::sync::CancellationToken;
use url::Url;

/// Upper bounds that hold regardless of user options.
const MAX_PAGES_HARD: u32 = 5_000;
const MAX_FILES: usize = 20_000;
const MAX_PAGE_BYTES: usize = 8 * 1024 * 1024;
const PER_HOST_CONCURRENCY: usize = 2;
const PER_HOST_MIN_DELAY: Duration = Duration::from_millis(250);

pub struct Crawler {
    clients: Arc<ClientFactory>,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
}

pub struct Session {
    pub id: String,
    state: Mutex<GrabberSession>,
    cancel: CancellationToken,
    progress_tx: watch::Sender<(u32, u32, bool)>,
}

impl Session {
    pub fn snapshot(&self) -> GrabberSession {
        self.state.lock().clone()
    }
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
    /// `(pages_crawled, files_found, done)` updates.
    pub fn progress(&self) -> watch::Receiver<(u32, u32, bool)> {
        self.progress_tx.subscribe()
    }
    fn touch(&self) {
        let s = self.state.lock();
        let _ = self
            .progress_tx
            .send_replace((s.pages_crawled, s.files.len() as u32, s.done));
    }
}

struct HostState {
    sem: Arc<Semaphore>,
    last: Mutex<Instant>,
    robots: tokio::sync::OnceCell<Robots>,
}

impl Crawler {
    pub fn new(clients: Arc<ClientFactory>) -> Arc<Self> {
        Arc::new(Self {
            clients,
            sessions: Mutex::new(HashMap::new()),
        })
    }

    pub fn list(&self) -> Vec<GrabberSession> {
        self.sessions
            .lock()
            .values()
            .map(|s| s.snapshot())
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions.lock().get(id).cloned()
    }

    pub fn remove(&self, id: &str) -> Option<Arc<Session>> {
        let s = self.sessions.lock().remove(id);
        if let Some(s) = &s {
            s.cancel();
        }
        s
    }

    /// Validate options and start crawling in the background.
    pub fn start(self: &Arc<Self>, options: GrabberOptions) -> Result<Arc<Session>, TaskError> {
        let start = Url::parse(options.url.trim())
            .map_err(|e| TaskError::new(ErrorKind::InvalidUrl, e.to_string()))?;
        if !matches!(start.scheme(), "http" | "https") {
            return Err(TaskError::new(
                ErrorKind::UnsupportedScheme,
                start.scheme().to_owned(),
            ));
        }
        let include_regex = match &options.include_regex {
            Some(p) if !p.trim().is_empty() => Some(
                regex::RegexBuilder::new(p)
                    .size_limit(1 << 20)
                    .build()
                    .map_err(|e| {
                        TaskError::new(ErrorKind::ParseError, format!("include regex: {e}"))
                    })?,
            ),
            _ => None,
        };
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, _rx) = watch::channel((0, 0, false));
        let session = Arc::new(Session {
            id: id.clone(),
            state: Mutex::new(GrabberSession {
                id: id.clone(),
                options: options.clone(),
                started_at: Millis::now(),
                ..Default::default()
            }),
            cancel: CancellationToken::new(),
            progress_tx: tx,
        });
        self.sessions.lock().insert(id, session.clone());
        let this = self.clone();
        let sess = session.clone();
        tokio::spawn(async move {
            let result = this
                .crawl(sess.clone(), start, options, include_regex)
                .await;
            let mut s = sess.state.lock();
            s.done = true;
            s.finished_at = Some(Millis::now());
            s.cancelled = sess.cancel.is_cancelled();
            if let Err(e) = result {
                s.error = Some(e.message);
            }
            drop(s);
            sess.touch();
        });
        Ok(session)
    }

    async fn crawl(
        &self,
        session: Arc<Session>,
        start: Url,
        opts: GrabberOptions,
        include_regex: Option<regex::Regex>,
    ) -> Result<(), TaskError> {
        let client = self.clients.client(&swoop_runtime::net::ClientProfile {
            cookies: true,
            ..Default::default()
        })?;
        let scope = Scope::parse(&opts.scope);
        let exts: Vec<String> = if opts.include_extensions.is_empty() {
            default_extensions()
        } else {
            opts.include_extensions
                .iter()
                .map(|e| e.trim_start_matches('.').to_ascii_lowercase())
                .collect()
        };
        let max_pages = opts.max_pages.clamp(1, MAX_PAGES_HARD);
        let concurrency = (opts.concurrency as usize).clamp(1, 16);
        let user_agent = self.clients.settings().network.user_agent.clone();
        let hosts: Arc<Mutex<HashMap<String, Arc<HostState>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let seen: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        let files_seen: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        let mut frontier: VecDeque<(Url, u8, String)> = VecDeque::new();
        seen.lock().insert(start.as_str().to_owned());
        frontier.push_back((start.clone(), 0, String::new()));
        let sem = Arc::new(Semaphore::new(concurrency));
        let mut in_flight = tokio::task::JoinSet::new();
        let mut crawled = 0u32;

        loop {
            if session.cancel.is_cancelled() {
                break;
            }
            // schedule as many as concurrency allows
            while crawled + (in_flight.len() as u32) < max_pages {
                let Some((url, depth, found_on)) = frontier.pop_front() else {
                    break;
                };
                let permit = match sem.clone().try_acquire_owned() {
                    Ok(p) => p,
                    Err(_) => {
                        frontier.push_front((url, depth, found_on));
                        break;
                    }
                };
                let host = url.host_str().unwrap_or("").to_owned();
                let hs = hosts
                    .lock()
                    .entry(host.clone())
                    .or_insert_with(|| {
                        Arc::new(HostState {
                            sem: Arc::new(Semaphore::new(PER_HOST_CONCURRENCY)),
                            last: Mutex::new(Instant::now() - PER_HOST_MIN_DELAY),
                            robots: tokio::sync::OnceCell::new(),
                        })
                    })
                    .clone();
                let client = client.clone();
                let cancel = session.cancel.clone();
                let ua = user_agent.clone();
                let respect_robots = opts.respect_robots;
                in_flight.spawn(async move {
                    let _permit = permit;
                    let r = fetch_page(&client, &hs, &url, &ua, respect_robots, cancel).await;
                    (url, depth, found_on, r)
                });
            }
            if in_flight.is_empty() {
                break;
            }
            let Some(joined) = in_flight.join_next().await else {
                break;
            };
            let Ok((url, depth, _found_on, result)) = joined else {
                continue;
            };
            {
                let mut s = session.state.lock();
                s.pages_queued = frontier.len() as u32;
            }
            match result {
                Ok(PageResult::RobotsBlocked) => {
                    session.state.lock().robots_blocked += 1;
                }
                Ok(PageResult::File { mime, size }) => {
                    // the "page" turned out to be a file
                    self.record_file(
                        &session,
                        &opts,
                        &exts,
                        include_regex.as_ref(),
                        &files_seen,
                        &url,
                        &url,
                        depth,
                        Some(mime),
                        size,
                    );
                }
                Ok(PageResult::Html { body, final_url }) => {
                    crawled += 1;
                    session.state.lock().pages_crawled = crawled;
                    let extracted = extract(&body);
                    let base = extracted
                        .base_href
                        .as_deref()
                        .and_then(|b| final_url.join(b).ok())
                        .unwrap_or(final_url.clone());
                    let mut candidates: Vec<(String, bool)> =
                        extracted.links.iter().map(|l| (l.clone(), false)).collect();
                    candidates.extend(extracted.media.iter().map(|m| (m.clone(), true)));
                    if opts.follow_iframes {
                        candidates.extend(extracted.iframes.iter().map(|i| (i.clone(), false)));
                    }
                    for (raw, is_media) in candidates {
                        let Some(u) = normalize(&base, &raw) else {
                            continue;
                        };
                        if is_media || !looks_like_page(&u) {
                            // file candidate: scope applies to files too unless external
                            if scope.allows(&start, &u) || scope == Scope::External {
                                self.record_file(
                                    &session,
                                    &opts,
                                    &exts,
                                    include_regex.as_ref(),
                                    &files_seen,
                                    &u,
                                    &final_url,
                                    depth,
                                    None,
                                    None,
                                );
                            }
                            continue;
                        }
                        if depth < opts.max_depth && scope.allows(&start, &u) {
                            let key = u.as_str().to_owned();
                            if seen.lock().insert(key) {
                                frontier.push_back((u, depth + 1, final_url.to_string()));
                            }
                        }
                    }
                    session.state.lock().pages_queued = frontier.len() as u32;
                }
                Err(e) => {
                    tracing::debug!(url = %swoop_runtime::redact::redact(url.as_str()), "grabber page failed: {}", e.message);
                    if crawled == 0 && frontier.is_empty() && in_flight.is_empty() {
                        return Err(e);
                    }
                }
            }
            session.touch();
            if session.state.lock().files.len() >= MAX_FILES {
                break;
            }
        }
        in_flight.shutdown().await;

        // probe files for size/MIME
        if opts.probe_files && !session.cancel.is_cancelled() {
            self.probe_files(&session, &client, &hosts, &opts).await;
        }
        // size filters (applied after probing when sizes are known)
        if opts.min_size.is_some() || opts.max_size.is_some() {
            let mut s = session.state.lock();
            s.files.retain(|f| match f.size {
                Some(sz) => {
                    opts.min_size.map(|m| sz >= m).unwrap_or(true)
                        && opts.max_size.map(|m| sz <= m).unwrap_or(true)
                }
                None => true,
            });
        }
        session.touch();
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn record_file(
        &self,
        session: &Session,
        opts: &GrabberOptions,
        exts: &[String],
        include_regex: Option<&regex::Regex>,
        files_seen: &Mutex<HashSet<String>>,
        url: &Url,
        found_on: &Url,
        depth: u8,
        mime: Option<String>,
        size: Option<u64>,
    ) {
        let kind = match accept_file(opts, url, exts, include_regex) {
            Some(k) => k,
            None => match mime.as_deref().and_then(kind_for_mime) {
                Some(k) if opts.include_extensions.is_empty() => k,
                _ => return,
            },
        };
        if !files_seen.lock().insert(url.as_str().to_owned()) {
            return;
        }
        let name =
            swoop_runtime::filename::from_url(url.as_str()).unwrap_or_else(|| "download".into());
        let f = GrabberFile {
            url: url.to_string(),
            name,
            extension: crate::classify::extension_of(url),
            domain: url.host_str().unwrap_or("").to_ascii_lowercase(),
            found_on: found_on.to_string(),
            size,
            mime,
            kind: kind.to_owned(),
            depth,
        };
        let mut s = session.state.lock();
        if s.files.len() < MAX_FILES {
            s.files.push(f);
        }
    }

    async fn probe_files(
        &self,
        session: &Arc<Session>,
        client: &reqwest::Client,
        hosts: &Arc<Mutex<HashMap<String, Arc<HostState>>>>,
        opts: &GrabberOptions,
    ) {
        let files: Vec<(usize, String)> = session
            .state
            .lock()
            .files
            .iter()
            .enumerate()
            .filter(|(_, f)| f.size.is_none() || f.mime.is_none())
            .map(|(i, f)| (i, f.url.clone()))
            .collect();
        let sem = Arc::new(Semaphore::new((opts.concurrency as usize).clamp(1, 16)));
        let mut set = tokio::task::JoinSet::new();
        for (i, url) in files {
            if session.cancel.is_cancelled() {
                break;
            }
            let Ok(u) = Url::parse(&url) else { continue };
            let host = u.host_str().unwrap_or("").to_owned();
            let hs = hosts
                .lock()
                .entry(host)
                .or_insert_with(|| {
                    Arc::new(HostState {
                        sem: Arc::new(Semaphore::new(PER_HOST_CONCURRENCY)),
                        last: Mutex::new(Instant::now() - PER_HOST_MIN_DELAY),
                        robots: tokio::sync::OnceCell::new(),
                    })
                })
                .clone();
            let permit = match sem.clone().acquire_owned().await {
                Ok(p) => p,
                Err(_) => break,
            };
            let client = client.clone();
            let cancel = session.cancel.clone();
            set.spawn(async move {
                let _p = permit;
                let work = async {
                    let _h = hs.sem.clone().acquire_owned().await.ok();
                    polite_wait(&hs).await;
                    let req = client.head(u.clone()).timeout(Duration::from_secs(15));
                    match req.send().await {
                        Ok(resp) if resp.status().is_success() => (
                            resp.headers()
                                .get(reqwest::header::CONTENT_LENGTH)
                                .and_then(|v| v.to_str().ok())
                                .and_then(|v| v.parse().ok()),
                            resp.headers()
                                .get(reqwest::header::CONTENT_TYPE)
                                .and_then(|v| v.to_str().ok())
                                .map(|v| v.split(';').next().unwrap_or("").trim().to_owned()),
                        ),
                        _ => (None, None),
                    }
                };
                let (size, mime) = tokio::select! {
                    r = work => r,
                    _ = cancel.cancelled() => (None, None),
                };
                (i, size, mime)
            });
        }
        while let Some(Ok((i, size, mime))) = set.join_next().await {
            let mut s = session.state.lock();
            if let Some(f) = s.files.get_mut(i) {
                if f.size.is_none() {
                    f.size = size;
                }
                if f.mime.is_none() {
                    f.mime = mime;
                }
            }
            drop(s);
            session.touch();
        }
    }
}

enum PageResult {
    Html { body: Vec<u8>, final_url: Url },
    File { mime: String, size: Option<u64> },
    RobotsBlocked,
}

async fn polite_wait(hs: &HostState) {
    let wait = {
        let mut last = hs.last.lock();
        let now = Instant::now();
        let elapsed = now.duration_since(*last);
        let wait = PER_HOST_MIN_DELAY.saturating_sub(elapsed);
        *last = now + wait;
        wait
    };
    if !wait.is_zero() {
        tokio::time::sleep(wait).await;
    }
}

async fn fetch_page(
    client: &reqwest::Client,
    hs: &Arc<HostState>,
    url: &Url,
    ua: &str,
    respect_robots: bool,
    cancel: CancellationToken,
) -> Result<PageResult, TaskError> {
    let _h = hs
        .sem
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| TaskError::internal("host semaphore"))?;
    if respect_robots {
        let robots = hs
            .robots
            .get_or_init(|| async {
                let mut r = url.clone();
                r.set_path("/robots.txt");
                r.set_query(None);
                match client.get(r).timeout(Duration::from_secs(10)).send().await {
                    Ok(resp) if resp.status().is_success() => match resp.text().await {
                        Ok(t) if t.len() < 512 * 1024 => Robots::parse(&t, ua),
                        _ => Robots::permissive(),
                    },
                    _ => Robots::permissive(),
                }
            })
            .await;
        let path = match url.query() {
            Some(q) => format!("{}?{}", url.path(), q),
            None => url.path().to_owned(),
        };
        if !robots.allows(&path) {
            return Ok(PageResult::RobotsBlocked);
        }
        if let Some(ms) = robots.crawl_delay_ms {
            let mut last = hs.last.lock();
            *last = (*last)
                .max(Instant::now() - PER_HOST_MIN_DELAY + Duration::from_millis(ms.min(10_000)));
        }
    }
    polite_wait(hs).await;
    let req = client
        .get(url.clone())
        .header(
            reqwest::header::ACCEPT,
            "text/html,application/xhtml+xml;q=0.9,*/*;q=0.5",
        )
        .timeout(Duration::from_secs(30));
    let resp = tokio::select! {
        r = req.send() => r.map_err(|e| TaskError::new(ErrorKind::ConnectionReset, e.to_string()))?,
        _ = cancel.cancelled() => return Err(TaskError::cancelled()),
    };
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(TaskError::from_http_status(status, url.as_str()));
    }
    let final_url = resp.url().clone();
    let mime = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        })
        .unwrap_or_default();
    let size = resp.content_length();
    if !(mime.contains("html") || mime.contains("xml") || mime.is_empty()) {
        return Ok(PageResult::File { mime, size });
    }
    let mut body =
        Vec::with_capacity(size.unwrap_or(16 * 1024).min(MAX_PAGE_BYTES as u64) as usize);
    let mut resp = resp;
    loop {
        let chunk = tokio::select! {
            c = resp.chunk() => c.map_err(|e| TaskError::new(ErrorKind::Truncated, e.to_string()))?,
            _ = cancel.cancelled() => return Err(TaskError::cancelled()),
        };
        let Some(chunk) = chunk else { break };
        body.extend_from_slice(&chunk);
        if body.len() > MAX_PAGE_BYTES {
            break;
        }
    }
    Ok(PageResult::Html { body, final_url })
}
