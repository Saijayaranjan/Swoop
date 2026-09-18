use osprey_domain::settings::Settings;
use osprey_grabber::{Crawler, GrabberOptions};
use osprey_runtime::net::ClientFactory;
use osprey_testserver::TestServer;
use std::sync::Arc;
use std::time::Duration;

async fn wait_done(session: &osprey_grabber::Session) -> osprey_grabber::GrabberSession {
    let mut rx = session.progress();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if session.snapshot().done {
            return session.snapshot();
        }
        tokio::select! {
            _ = rx.changed() => {},
            _ = tokio::time::sleep_until(deadline) => panic!("crawl did not finish"),
        }
    }
}

fn site(server: &TestServer) {
    let base = server.url("");
    server.add_html(
        "index",
        &format!(
            r#"<html><head><title>Root</title></head><body>
        <a href="/html/docs">Docs</a>
        <a href="/html/private/secret">Secret</a>
        <a href="/file/a.pdf?size=3000">A</a>
        <a href="{base}/file/b.zip?size=5000&utm_source=x">B</a>
        <a href="https://external.invalid/ext.pdf">External</a>
        <img src="/file/pic.jpg?size=100" srcset="/file/pic2.jpg?size=200 2x">
        <a href="/file/thumb_small.png?size=10">thumb</a>
        </body></html>"#
        ),
    );
    server.add_html("docs", r#"<html><body><a href="/html/deep">Deep</a><a href="/file/report.PDF?size=7000">Report</a><a href="/file/a.pdf?size=3000">dup</a></body></html>"#);
    server.add_html(
        "deep",
        r#"<html><body><a href="/file/deep.mp4?size=9000">Deep video</a></body></html>"#,
    );
    server.add_html(
        "private/secret",
        r#"<html><body><a href="/file/secret.pdf?size=1">S</a></body></html>"#,
    );
    server.add_text(
        "robots.txt",
        "text/plain",
        "User-agent: *\nDisallow: /html/private/\n",
    );
}

#[tokio::test]
async fn crawls_with_depth_scope_robots_and_filters() {
    let server = TestServer::start().await;
    site(&server);
    let clients = ClientFactory::new(Arc::new(Settings::default()));
    let crawler = Crawler::new(clients);
    let opts = GrabberOptions {
        url: server.url("/html/index"),
        max_depth: 1,
        exclude_patterns: vec!["*thumb*".into()],
        probe_files: true,
        ..Default::default()
    };
    let session = crawler.start(opts).unwrap();
    let s = wait_done(&session).await;
    assert!(s.error.is_none(), "{:?}", s.error);
    // index + docs crawled (depth 1); deep is depth 2 → not crawled; private blocked by robots
    assert_eq!(s.pages_crawled, 2, "{s:?}");
    assert_eq!(s.robots_blocked, 1);
    let urls: Vec<&str> = s.files.iter().map(|f| f.url.as_str()).collect();
    assert!(urls.iter().any(|u| u.contains("/file/a.pdf")));
    assert!(urls
        .iter()
        .any(|u| u.contains("/file/b.zip") && !u.contains("utm_source")));
    assert!(urls.iter().any(|u| u.contains("/file/report.PDF")));
    assert!(urls.iter().any(|u| u.contains("/file/pic2.jpg")));
    assert!(
        !urls.iter().any(|u| u.contains("thumb")),
        "excluded pattern"
    );
    assert!(!urls.iter().any(|u| u.contains("deep.mp4")), "beyond depth");
    assert!(
        !urls.iter().any(|u| u.contains("external.invalid")),
        "out of scope"
    );
    assert!(!urls.iter().any(|u| u.contains("secret")), "robots");
    // no duplicates
    let mut sorted = urls.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), urls.len());
    // probed sizes and kinds
    let a = s.files.iter().find(|f| f.url.contains("a.pdf")).unwrap();
    assert_eq!(a.size, Some(3000));
    assert_eq!(a.kind, "document");
    assert_eq!(a.extension, "pdf");
    assert_eq!(a.name, "a.pdf");
    let v = s.files.iter().find(|f| f.url.contains("b.zip")).unwrap();
    assert_eq!(v.kind, "archive");
}

#[tokio::test]
async fn external_scope_and_extension_filter_and_size_filter() {
    let server = TestServer::start().await;
    site(&server);
    let clients = ClientFactory::new(Arc::new(Settings::default()));
    let crawler = Crawler::new(clients);
    let opts = GrabberOptions {
        url: server.url("/html/index"),
        max_depth: 2,
        scope: "external".into(),
        include_extensions: vec!["pdf".into(), "mp4".into()],
        min_size: Some(2000),
        respect_robots: false,
        ..Default::default()
    };
    let session = crawler.start(opts).unwrap();
    let s = wait_done(&session).await;
    let urls: Vec<&str> = s.files.iter().map(|f| f.url.as_str()).collect();
    assert!(
        urls.iter().any(|u| u.contains("deep.mp4")),
        "depth 2 reached: {urls:?}"
    );
    assert!(
        urls.iter().any(|u| u.contains("external.invalid/ext.pdf")),
        "external allowed"
    );
    assert!(!urls.iter().any(|u| u.contains(".zip")), "extension filter");
    assert!(
        !urls.iter().any(|u| u.contains("secret.pdf")),
        "size filter (1 byte) removed it"
    );
    assert_eq!(s.robots_blocked, 0);
    assert_eq!(s.pages_crawled, 4);
}

#[tokio::test]
async fn cancel_stops_quickly() {
    let server = TestServer::start().await;
    // a page whose links are slow files; the crawl itself is fast, so test cancel during probing
    let mut html = String::from("<html><body>");
    for i in 0..40 {
        html.push_str(&format!(
            "<a href=\"/file/f{i}.pdf?size=1000&delay_ms=500\">f</a>"
        ));
    }
    html.push_str("</body></html>");
    server.add_html("slow", &html);
    let clients = ClientFactory::new(Arc::new(Settings::default()));
    let crawler = Crawler::new(clients);
    let session = crawler
        .start(GrabberOptions {
            url: server.url("/html/slow"),
            max_depth: 0,
            concurrency: 2,
            ..Default::default()
        })
        .unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;
    let t0 = std::time::Instant::now();
    session.cancel();
    let s = wait_done(&session).await;
    assert!(t0.elapsed() < Duration::from_secs(3));
    assert!(s.cancelled);
    assert_eq!(s.files.len(), 40);
    assert!(crawler.get(&session.id).is_some());
    crawler.remove(&session.id);
    assert!(crawler.get(&session.id).is_none());
}

#[tokio::test]
async fn rejects_bad_input() {
    let clients = ClientFactory::new(Arc::new(Settings::default()));
    let crawler = Crawler::new(clients);
    assert!(crawler
        .start(GrabberOptions {
            url: "ftp://x/".into(),
            ..Default::default()
        })
        .is_err());
    assert!(crawler
        .start(GrabberOptions {
            url: "http://x/".into(),
            include_regex: Some("(".into()),
            ..Default::default()
        })
        .is_err());
}
