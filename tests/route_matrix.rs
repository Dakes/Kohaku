//! Walks the route table and checks every entry on the wire against its declarations
//! (http-security: Per-route security declarations; change foundation D12). Matches
//! on declaration enums have no wildcard, so a new variant fails to compile here.

mod support;

use axum::body::Body;
use axum::http::StatusCode;
use kohaku::http::headers::{CSP, fixed_headers};
use kohaku::limits::rate::RateClass;
use kohaku::routing::hosts::{HostMap, ProjectId};
use kohaku::routing::table::{
    Access, BodyClass, CacheClass, Csp, ErrorFormat, Headerless, HostKind, Multipart, Route,
    RoutePath, table,
};
use support::*;

const PROJECT_HOST: &str = "bugs.example.net";

/// Routes allowed to set their own CSP or Cache-Control: none in this change.
const OWN_HEADER_ALLOWLIST: &[&str] = &[];

fn harness() -> Harness {
    let harness = Harness::new();
    harness
        .app
        .host_map
        .replace(HostMap::new(&harness.app.base_url).with_project(PROJECT_HOST, ProjectId(1)));
    harness
}

/// Hosts an entry is requested on.
fn hosts(kind: HostKind) -> Vec<&'static str> {
    match kind {
        HostKind::Internal => vec![MAIN_HOST, PROJECT_HOST, "evil.example"],
        HostKind::Main => vec![MAIN_HOST],
        HostKind::Project => vec![PROJECT_HOST],
        HostKind::Both => vec![MAIN_HOST, PROJECT_HOST],
    }
}

fn origin(host: &str) -> String {
    format!("https://{host}")
}

fn expected_cache(class: CacheClass, status: StatusCode, method: &str) -> &'static str {
    match class {
        CacheClass::NoStore => "no-store",
        CacheClass::NoCache => "no-cache",
        CacheClass::StaticAsset if status == StatusCode::OK && method != "POST" => {
            "public, max-age=31536000, immutable"
        }
        CacheClass::StaticAsset => "no-cache",
    }
}

fn check_headers(response: &axum::http::Response<Body>, route: &Route, method: &str, what: &str) {
    for (name, value) in fixed_headers() {
        assert_eq!(
            header_values(response, name.as_str()),
            [value],
            "{what}: {name}"
        );
    }
    match route.csp {
        Csp::Release => assert_eq!(
            header_values(response, "content-security-policy"),
            [CSP],
            "{what}"
        ),
    }
    assert_eq!(
        header_values(response, "cache-control"),
        [expected_cache(route.cache, response.status(), method)],
        "{what}"
    );
    assert!(header_values(response, "set-cookie").is_empty(), "{what}");
    assert!(
        !response
            .headers()
            .keys()
            .any(|k| k.as_str().starts_with("access-control-allow-")),
        "{what}"
    );
}

fn error_content_type(format: ErrorFormat) -> &'static str {
    match format {
        ErrorFormat::Html => "text/html; charset=utf-8",
        ErrorFormat::Json => "application/json",
    }
}

#[tokio::test]
async fn route_table_matches_its_declarations() {
    let routes = table();
    assert!(
        routes
            .iter()
            .any(|r| r.path == RoutePath::Fallback && r.host == HostKind::Main)
    );
    assert!(
        routes
            .iter()
            .any(|r| r.path == RoutePath::Fallback && r.host == HostKind::Project)
    );
    for route in &routes {
        let label = format!("{:?} {:?}", route.host, route.path);
        match route.access {
            Access::Public => {}
        }
        match route.csp {
            Csp::Release => {}
        }
        assert!(!OWN_HEADER_ALLOWLIST.contains(&label.as_str()));
        match route.body {
            BodyClass::Default => {
                assert_eq!(route.body.cap(), 65_536);
                assert_eq!(route.body.deadline().as_secs(), 15);
            }
        }
        match route.multipart {
            Multipart::Rejected => {}
        }
        match route.headerless {
            Headerless::NotExempt => {}
            Headerless::Exempt => panic!("{label}: no route of this change is exempt"),
        }
        for class in route.rate {
            match class {
                RateClass::Read => {}
            }
        }
        for host in hosts(route.host) {
            // Each request gets a fresh app, so buckets start full.
            let harness = harness();
            for method in ["GET", "HEAD"] {
                let what = format!("{label} {method} {host}");
                let response = harness
                    .send(
                        request(method, host, &route.example)
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await;
                check_headers(&response, route, method, &what);
                if response.status().is_client_error() {
                    assert_eq!(
                        header_values(&response, "content-type"),
                        [error_content_type(route.errors)],
                        "{what}"
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn state_changing_requests_need_the_origin() {
    for route in table() {
        let label = format!("{:?} {:?}", route.host, route.path);
        for host in hosts(route.host) {
            let harness = harness();
            let cross_site = [
                vec![("origin", "null".to_owned())],
                vec![("sec-fetch-site", "cross-site".to_owned())],
                vec![],
            ];
            for headers in cross_site {
                let mut builder = request("POST", host, &route.example);
                for (name, value) in &headers {
                    builder = builder.header(*name, value);
                }
                let response = harness.send(builder.body(Body::empty()).unwrap()).await;
                let what = format!("{label} POST {host} {headers:?}");
                check_headers(&response, &route, "POST", &what);
                match route.host {
                    HostKind::Internal => {
                        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED, "{what}")
                    }
                    HostKind::Main | HostKind::Project | HostKind::Both => {
                        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{what}")
                    }
                }
            }
            let same_origin = harness
                .send(
                    request("POST", host, &route.example)
                        .header("origin", origin(host))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await;
            let status = same_origin.status();
            assert!(
                status == StatusCode::METHOD_NOT_ALLOWED || status == StatusCode::NOT_FOUND,
                "{label} same-origin POST {host}: {status}"
            );
        }
    }
}

#[tokio::test]
async fn caps_multipart_rate_and_unknown_hosts() {
    for route in table() {
        let label = format!("{:?} {:?}", route.host, route.path);
        if route.host == HostKind::Internal {
            continue;
        }
        for host in hosts(route.host) {
            let harness = harness();
            let oversize = harness
                .send(
                    request("POST", host, &route.example)
                        .header("origin", origin(host))
                        .header("content-length", "65537")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await;
            assert_eq!(
                oversize.status(),
                StatusCode::PAYLOAD_TOO_LARGE,
                "{label} {host}"
            );
            check_headers(&oversize, &route, "POST", &label);

            let multipart = harness
                .send(
                    request("GET", host, &route.example)
                        .header("content-type", "multipart/form-data; boundary=x")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await;
            assert_eq!(
                multipart.status(),
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "{label} {host}"
            );
            check_headers(&multipart, &route, "GET", &label);

            let unknown = harness.get("evil.example", &route.example).await;
            assert_eq!(unknown.status(), StatusCode::MISDIRECTED_REQUEST, "{label}");

            // Empty the read bucket of the default peer, then request once more.
            let limited = route.rate.contains(&RateClass::Read);
            let mut last = StatusCode::OK;
            for _ in 0..301 {
                last = harness.get(host, &route.example).await.status();
            }
            if limited {
                assert_eq!(last, StatusCode::TOO_MANY_REQUESTS, "{label} {host}");
                let response = harness.get(host, &route.example).await;
                check_headers(&response, &route, "GET", &label);
            } else {
                assert_ne!(last, StatusCode::TOO_MANY_REQUESTS, "{label} {host}");
            }
        }
    }
}
