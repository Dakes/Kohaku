//! Host routing on the wire (host-routing; change foundation D15).

mod support;

use axum::body::Body;
use axum::http::StatusCode;
use kohaku::assets;
use kohaku::routing::hosts::{HostMap, ProjectId};
use support::*;

const PROJECT_HOST: &str = "bugs.example.net";

fn with_project(harness: &Harness) {
    harness
        .app
        .host_map
        .replace(HostMap::new(&harness.app.base_url).with_project(PROJECT_HOST, ProjectId(1)));
}

fn stylesheet() -> &'static str {
    assets::path("kohaku.css")
}

#[tokio::test]
async fn mixed_case_and_a_port_are_normalized() {
    let harness = Harness::with(
        kohaku::routing::table::table(),
        &[("KOHAKU_BASE_URL", Some("https://kohaku.example.org:8443"))],
    );
    let response = harness.get("Kohaku.Example.ORG:8080", "/").await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn malformed_repeated_or_missing_host_gets_421() {
    let harness = Harness::new();
    for host in [
        "kohaku.example.org.",
        "kohaku.example.org:x",
        "evil.example@kohaku.example.org",
    ] {
        assert_eq!(
            harness.get(host, "/").await.status(),
            StatusCode::MISDIRECTED_REQUEST,
            "{host}"
        );
    }
    let two = request("GET", MAIN_HOST, "/")
        .header("host", "evil.example")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        harness.send(two).await.status(),
        StatusCode::MISDIRECTED_REQUEST
    );
    let none = request("GET", "", "/").body(Body::empty()).unwrap();
    assert_eq!(
        harness.send(none).await.status(),
        StatusCode::MISDIRECTED_REQUEST
    );
}

#[tokio::test]
async fn forwarded_headers_are_ignored() {
    let harness = Harness::new();
    let forged = request("GET", "evil.example", "/")
        .header("x-forwarded-host", MAIN_HOST)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        harness.send(forged).await.status(),
        StatusCode::MISDIRECTED_REQUEST
    );
    let plain = body_text(harness.get(MAIN_HOST, "/").await).await;
    let with_headers = request("GET", MAIN_HOST, "/")
        .header("x-forwarded-host", "evil.example")
        .header("forwarded", "host=evil.example;proto=http")
        .body(Body::empty())
        .unwrap();
    let response = harness.send(with_headers).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_text(response).await, plain);
}

#[tokio::test]
async fn lookalike_and_random_hosts_get_421() {
    let harness = Harness::new();
    for host in [
        "evil.kohaku.example.org",
        "kohaku.example.org.evil.example",
        PROJECT_HOST,
    ] {
        assert_eq!(
            harness.get(host, "/").await.status(),
            StatusCode::MISDIRECTED_REQUEST
        );
    }
    for n in 0..10_000u32 {
        let host = format!("h{n:x}.random{}.test", n % 97);
        assert_eq!(
            harness.get(&host, "/").await.status(),
            StatusCode::MISDIRECTED_REQUEST
        );
    }
}

#[tokio::test]
async fn unknown_host_body_is_fixed_and_never_echoes_the_host() {
    let harness = Harness::new();
    let reference = body_text(harness.get("other.example", "/").await).await;
    for (method, path) in [
        ("GET", "/"),
        ("GET", "/admin"),
        ("POST", "/"),
        ("GET", stylesheet()),
    ] {
        let response = harness
            .send(
                request(method, "evil.example", path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(
            response.status(),
            StatusCode::MISDIRECTED_REQUEST,
            "{method} {path}"
        );
        assert!(header_values(&response, "set-cookie").is_empty());
        assert!(response.headers().get("location").is_none());
        let body = body_text(response).await;
        assert!(!body.contains("evil.example"));
        assert_eq!(body, reference);
    }
    for host in ["kohaku:8080", "203.0.113.7"] {
        assert_eq!(
            harness.get(host, "/").await.status(),
            StatusCode::MISDIRECTED_REQUEST
        );
    }
}

#[tokio::test]
async fn internal_endpoints_answer_any_host() {
    let harness = Harness::new();
    for host in ["127.0.0.1:8080", "kohaku:8080", "evil.example", ""] {
        let response = harness
            .send(
                request("GET", host, "/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK, "{host:?}");
        let ask = harness
            .send(
                request("GET", host, "/.well-known/kohaku/tls-ask?domain=x.test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(ask.status(), StatusCode::NOT_FOUND, "{host:?}");
    }
}

#[tokio::test]
async fn probes_around_the_internal_endpoints() {
    let harness = Harness::new();
    for path in [
        "/.well-known/kohaku/other",
        "/healthz/x",
        "/healthz/",
        "/HEALTHZ",
    ] {
        assert_eq!(
            harness.get("evil.example", path).await.status(),
            StatusCode::MISDIRECTED_REQUEST,
            "{path}"
        );
    }
    for path in ["/healthz", "/.well-known/kohaku/tls-ask"] {
        for method in ["POST", "PUT", "DELETE", "OPTIONS"] {
            let response = harness
                .send(
                    request(method, "evil.example", path)
                        .header("sec-fetch-site", "cross-site")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await;
            assert_eq!(
                response.status(),
                StatusCode::METHOD_NOT_ALLOWED,
                "{method} {path}"
            );
        }
    }
}

#[tokio::test]
async fn tls_ask_answers_only_for_project_hosts() {
    let harness = Harness::new();
    with_project(&harness);
    let ask = |query: &'static str| {
        let harness = &harness;
        async move {
            harness
                .send(
                    request(
                        "GET",
                        "kohaku:8080",
                        &format!("/.well-known/kohaku/tls-ask{query}"),
                    )
                    .body(Body::empty())
                    .unwrap(),
                )
                .await
        }
    };
    assert_eq!(
        ask("?domain=Bugs.Example.NET:443").await.status(),
        StatusCode::OK
    );
    let not_found = body_text(ask("?domain=evil.example").await).await;
    for query in [
        "?domain=kohaku.example.org",
        "?domain=evil.example",
        "",
        "?domain=",
        "?domain=bugs.example.net&domain=evil.example",
    ] {
        let response = ask(query).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{query}");
        assert_eq!(body_text(response).await, not_found);
    }
    assert!(!not_found.contains("evil.example"));
}

#[tokio::test]
async fn attacker_enumerates_names() {
    let harness = Harness::new();
    let mut bodies = std::collections::HashSet::new();
    for n in 0..10_000u32 {
        let response = harness
            .send(
                request(
                    "GET",
                    "kohaku:8080",
                    &format!("/.well-known/kohaku/tls-ask?domain=n{n}.test"),
                )
                .body(Body::empty())
                .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        bodies.insert(body_text(response).await);
    }
    assert_eq!(bodies.len(), 1);
}

#[tokio::test]
async fn attacker_probes_for_the_admin_area() {
    let harness = Harness::new();
    let reference = harness.get(MAIN_HOST, "/does-not-exist").await;
    assert_eq!(reference.status(), StatusCode::NOT_FOUND);
    let reference = body_text(reference).await;
    for path in [
        "/admin",
        "/admin/login",
        "/p/demo",
        "/p/demo/api/v1/reports",
    ] {
        let response = harness.get(MAIN_HOST, path).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        assert_eq!(body_text(response).await, reference, "{path}");
    }
}

#[tokio::test]
async fn landing_page_reflects_nothing() {
    let harness = Harness::new();
    let plain = body_text(harness.get(MAIN_HOST, "/").await).await;
    let response = harness
        .send(
            request("GET", MAIN_HOST, "/?q=%3Cscript%3Ealert(1)%3C/script%3E")
                .header("x-forwarded-host", "evil.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header_values(&response, "content-type"),
        ["text/html; charset=utf-8"]
    );
    assert!(header_values(&response, "set-cookie").is_empty());
    assert_eq!(body_text(response).await, plain);
}

#[tokio::test]
async fn post_to_the_landing_page_gets_405() {
    let harness = Harness::new();
    let response = harness
        .send(
            request("POST", MAIN_HOST, "/")
                .header("origin", "https://kohaku.example.org")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn project_host_serves_no_other_project_or_admin_area() {
    let harness = Harness::new();
    with_project(&harness);
    for path in ["/p/b", "/p/b/api/v1/reports", "/admin", "/admin/login", "/"] {
        assert_eq!(
            harness.get(PROJECT_HOST, path).await.status(),
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
}

#[tokio::test]
async fn stylesheet_on_both_hosts() {
    let harness = Harness::new();
    with_project(&harness);
    let main = harness.get(MAIN_HOST, stylesheet()).await;
    let project = harness.get(PROJECT_HOST, stylesheet()).await;
    for response in [&main, &project] {
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(header_values(response, "content-type"), ["text/css"]);
    }
    assert_eq!(body_text(main).await, body_text(project).await);
}

#[tokio::test]
async fn stale_or_traversal_paths_get_404() {
    let harness = Harness::new();
    let path = stylesheet();
    let flipped = {
        let mut bytes = path.as_bytes().to_vec();
        let i = "/static/kohaku.".len();
        bytes[i] = if bytes[i] == b'0' { b'1' } else { b'0' };
        String::from_utf8(bytes).unwrap()
    };
    for path in [
        flipped.as_str(),
        "/static/../../data/kohaku.db",
        "/static/%2e%2e%2f%2e%2e%2fdata%2fkohaku.db",
        "/static/",
        "/static/kohaku.css",
    ] {
        assert_eq!(
            harness.get(MAIN_HOST, path).await.status(),
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
}

#[tokio::test]
async fn no_cookies_even_for_a_forged_one() {
    let harness = Harness::new();
    with_project(&harness);
    for (host, path) in [
        (MAIN_HOST, "/"),
        (MAIN_HOST, stylesheet()),
        (MAIN_HOST, "/does-not-exist"),
        (PROJECT_HOST, stylesheet()),
        (PROJECT_HOST, "/x"),
        ("evil.example", "/"),
        ("evil.example", "/healthz"),
        (
            "evil.example",
            "/.well-known/kohaku/tls-ask?domain=bugs.example.net",
        ),
    ] {
        let response = harness
            .send(
                request("GET", host, path)
                    .header("cookie", "__Host-kohaku_session=forged")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert!(
            header_values(&response, "set-cookie").is_empty(),
            "{host} {path}"
        );
    }
}

#[tokio::test]
async fn unknown_hosts_are_counted() {
    let harness = Harness::new();
    for n in 0..1000 {
        harness.get(&format!("h{n}.test"), "/").await;
    }
    let counts = harness.app.counters.take();
    assert_eq!(counts, [(kohaku::logging::Reason::UnknownHost, 1000)]);
}

#[cfg(not(feature = "dev"))]
#[tokio::test]
async fn release_build_serves_no_development_route() {
    let harness = Harness::new();
    with_project(&harness);
    for host in [MAIN_HOST, PROJECT_HOST] {
        for path in ["/dev/boot-id", "/static/dev-reload.js"] {
            assert_eq!(
                harness.get(host, path).await.status(),
                StatusCode::NOT_FOUND,
                "{host} {path}"
            );
        }
    }
    let html = body_text(harness.get(MAIN_HOST, "/").await).await;
    assert!(!html.contains("dev-reload"));
}

#[tokio::test]
async fn error_bodies_never_echo_the_request() {
    let harness = Harness::new();
    let response = harness
        .send(
            request("GET", MAIN_HOST, "/alice@example.com/s3cret?token=t0ken")
                .header("x-forwarded-for", "203.0.113.77")
                .header("user-agent", "agent-x1")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = body_text(response).await;
    for marker in ["alice", "s3cret", "t0ken", "203.0.113.77", "agent-x1"] {
        assert!(!body.contains(marker), "{marker}");
    }
}
