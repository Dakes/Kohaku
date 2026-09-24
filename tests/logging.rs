//! What reaches the log (operations: Log content privacy, Health endpoint;
//! http-security: Request tracing records only route pattern and status; D21).

mod support;

use axum::body::Body;
use axum::http::StatusCode;
use axum::routing::get;
use kohaku::assets;
use kohaku::routing::hosts::{HostMap, ProjectId};
use kohaku::routing::table::{HostKind, table};
use support::*;

/// Distinctive values an attacker puts into requests.
const MARKERS: &[&str] = &[
    "alice@example.com",
    "s3cret",
    "c00kie",
    "t0ken",
    "203.0.113.77",
    "agent-x1",
    "r3f",
    "b0dytext",
];

fn marked(method: &str, host: &str, path: &str) -> axum::http::Request<Body> {
    request(method, host, path)
        .header("cookie", "__Host-kohaku_session=c00kie")
        .header("authorization", "Bearer t0ken")
        .header("x-forwarded-for", "203.0.113.77")
        .header("user-agent", "agent-x1")
        .header("referer", "https://evil.example/r3f")
        .header("origin", "https://kohaku.example.org")
        .body(Body::from("b0dytext alice@example.com"))
        .unwrap()
}

fn assert_clean(log: &str) {
    for marker in MARKERS {
        assert!(!log.contains(marker), "log contains {marker}:\n{log}");
    }
    assert!(!log.contains("?"), "log contains a query:\n{log}");
}

#[tokio::test(flavor = "current_thread")]
async fn tracing_records_only_the_pattern_and_status() {
    let capture = LogCapture::default();
    let _guard = capture.install();
    let mut routes = table();
    routes.push(synthetic_route(
        HostKind::Main,
        "/items/{id}",
        get(|| async { "item" }),
    ));
    let harness = Harness::with(routes, &[]);
    let stylesheet = format!(
        "{}?email=alice@example.com&token=s3cret",
        assets::path("kohaku.css")
    );
    let response = harness.send(marked("GET", MAIN_HOST, &stylesheet)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = harness
        .send(marked("GET", MAIN_HOST, "/alice@example.com/s3cret"))
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = harness
        .send(marked("GET", MAIN_HOST, "/items/alice@example.com"))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let log = capture.text();
    assert_clean(&log);
    assert!(log.contains("/static/{name}"), "{log}");
    assert!(log.contains("/items/{id}"), "{log}");
    assert!(
        log.contains("status=200") && log.contains("status=404"),
        "{log}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn attacker_data_never_reaches_the_log_on_any_entry() {
    let capture = LogCapture::default();
    let _guard = capture.install();
    let harness = Harness::with(table(), &[("KOHAKU_TRUSTED_PROXIES", Some("10.231.7.2"))]);
    harness.app.host_map.replace(
        HostMap::new(&harness.app.base_url).with_project("bugs.example.net", ProjectId(1)),
    );
    for route in table() {
        let hosts: &[&str] = match route.host {
            HostKind::Internal => &[MAIN_HOST, "evil.example"],
            HostKind::Main => &[MAIN_HOST, "evil.example"],
            HostKind::Project => &["bugs.example.net"],
            HostKind::Both => &[MAIN_HOST, "bugs.example.net"],
        };
        for host in hosts {
            for method in ["GET", "HEAD", "POST", "PUT"] {
                let path = format!("{}?email=alice@example.com&token=s3cret", route.example);
                let path = if path.contains("?domain=") {
                    path.replacen('?', "?x=1&", 1)
                } else {
                    path
                };
                harness.send(marked(method, host, &path)).await;
                harness
                    .send(from_peer(marked(method, host, &path), "10.231.7.2:4000"))
                    .await;
            }
        }
    }
    harness.app.counters.flush();
    assert_clean(&capture.text());
}

#[tokio::test(flavor = "current_thread")]
async fn health_endpoint_reflects_nothing() {
    let capture = LogCapture::default();
    let _guard = capture.install();
    let harness = Harness::new();
    for host in [MAIN_HOST, "unknown.example", ""] {
        for method in ["GET", "HEAD"] {
            let response = harness
                .send(
                    request(method, host, "/healthz?probe=%3Cscript%3Ex%3C/script%3E")
                        .header("x-forwarded-for", "203.0.113.77")
                        .header("cookie", "c00kie")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                header_values(&response, "content-type"),
                ["application/json"]
            );
            let headers = format!("{:?}", response.headers());
            let body = body_text(response).await;
            match method {
                "GET" => assert_eq!(body, r#"{"status":"ok"}"#),
                _ => assert!(body.is_empty()),
            }
            for text in [&headers, &body] {
                for marker in [
                    "<script",
                    "x</script",
                    "203.0.113.77",
                    "c00kie",
                    "unknown.example",
                    kohaku::cli::VERSION,
                ] {
                    assert!(!text.contains(marker), "{text}");
                }
            }
        }
    }
    assert!(capture.text().is_empty(), "{}", capture.text());
}

#[tokio::test(flavor = "current_thread")]
async fn flood_produces_one_line_per_reason() {
    let capture = LogCapture::default();
    let _guard = capture.install();
    let harness = Harness::new();
    for n in 0..1000 {
        harness.get(&format!("u{n}.example"), "/").await;
    }
    harness.app.counters.flush();
    let log = capture.text();
    assert_eq!(log.lines().count(), 1, "{log}");
    assert!(log.contains("unknown host (421): 1000"), "{log}");
    assert!(!log.contains("u1.example"));
}
