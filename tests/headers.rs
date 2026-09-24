//! Security headers and Cache-Control classes on every kind of response
//! (http-security; change foundation D13). Exact values are the release build's.

#![cfg(not(feature = "dev"))]

mod support;

use std::time::Duration;

use axum::body::Body;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use kohaku::assets;
use kohaku::http::headers::{RELEASE_CSP, fixed_headers};
use kohaku::routing::table::{HostKind, table};
use support::*;

const EXACT_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'";

fn assert_header_set(response: &axum::http::Response<Body>, what: &str) {
    assert_eq!(
        header_values(response, "content-security-policy"),
        [EXACT_CSP],
        "{what}"
    );
    for (name, value) in fixed_headers() {
        assert_eq!(
            header_values(response, name.as_str()),
            [value],
            "{what}: {name}"
        );
    }
    assert_eq!(header_values(response, "referrer-policy"), ["same-origin"]);
    assert_eq!(header_values(response, "x-frame-options"), ["DENY"]);
    assert_eq!(
        header_values(response, "cross-origin-opener-policy"),
        ["same-origin"]
    );
    assert_eq!(
        header_values(response, "x-content-type-options"),
        ["nosniff"]
    );
    assert_eq!(header_values(response, "permissions-policy"), [""]);
    assert_eq!(
        header_values(response, "strict-transport-security"),
        ["max-age=31536000"]
    );
    assert_eq!(header_values(response, "cache-control").len(), 1, "{what}");
}

/// A handler that tries to weaken every header.
async fn weakening() -> axum::response::Response {
    (
        [
            (header::CONTENT_SECURITY_POLICY, "default-src *"),
            (header::X_FRAME_OPTIONS, "ALLOWALL"),
            (header::REFERRER_POLICY, "unsafe-url"),
            (header::CACHE_CONTROL, "public, max-age=99"),
        ],
        "x",
    )
        .into_response()
}

async fn slow() -> &'static str {
    tokio::time::sleep(Duration::from_secs(20)).await;
    "late"
}

#[test]
fn release_csp_is_exact() {
    assert_eq!(RELEASE_CSP, EXACT_CSP);
}

#[tokio::test]
async fn every_response_carries_the_header_set() {
    let mut routes = table();
    routes.push(synthetic_route(
        HostKind::Main,
        "/weakening",
        get(weakening),
    ));
    routes.push(synthetic_route(HostKind::Main, "/slow", get(slow)));
    routes.push(synthetic_route(
        HostKind::Main,
        "/echo",
        post(|body: String| async move { body }),
    ));
    let harness = Harness::with(routes, &[]);
    let origin = "https://kohaku.example.org";
    let cases = [
        ("GET /", request("GET", MAIN_HOST, "/"), StatusCode::OK),
        ("HEAD /", request("HEAD", MAIN_HOST, "/"), StatusCode::OK),
        ("healthz", request("GET", "", "/healthz"), StatusCode::OK),
        (
            "tls-ask",
            request(
                "GET",
                "kohaku:8080",
                "/.well-known/kohaku/tls-ask?domain=x.test",
            ),
            StatusCode::NOT_FOUND,
        ),
        (
            "421",
            request("GET", "evil.example", "/"),
            StatusCode::MISDIRECTED_REQUEST,
        ),
        (
            "403 origin",
            request("POST", MAIN_HOST, "/").header("sec-fetch-site", "cross-site"),
            StatusCode::FORBIDDEN,
        ),
        (
            "403 isolation",
            request("GET", MAIN_HOST, "/admin").header("sec-fetch-site", "cross-site"),
            StatusCode::FORBIDDEN,
        ),
        (
            "404",
            request("GET", MAIN_HOST, "/nothing"),
            StatusCode::NOT_FOUND,
        ),
        (
            "405",
            request("POST", MAIN_HOST, "/").header("origin", origin),
            StatusCode::METHOD_NOT_ALLOWED,
        ),
        (
            "413",
            request("POST", MAIN_HOST, "/echo")
                .header("origin", origin)
                .header("content-length", "65537"),
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
        (
            "415",
            request("POST", MAIN_HOST, "/")
                .header("origin", origin)
                .header("content-type", "multipart/form-data; boundary=x"),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (
            "weakening",
            request("GET", MAIN_HOST, "/weakening"),
            StatusCode::OK,
        ),
    ];
    for (what, builder, status) in cases {
        let response = harness.send(builder.body(Body::empty()).unwrap()).await;
        assert_eq!(response.status(), status, "{what}");
        assert_header_set(&response, what);
    }
    let weakened = harness.get(MAIN_HOST, "/weakening").await;
    assert_eq!(header_values(&weakened, "cache-control"), ["no-store"]);

    let started = std::time::Instant::now();
    let late = harness.get(MAIN_HOST, "/slow").await;
    assert_eq!(late.status(), StatusCode::REQUEST_TIMEOUT);
    assert!(started.elapsed() >= Duration::from_secs(15));
    assert!(started.elapsed() < Duration::from_secs(17));
    assert_header_set(&late, "408");
}

#[tokio::test]
async fn each_response_gets_its_class() {
    let harness = Harness::new();
    let stylesheet = assets::path("kohaku.css");
    let expect = [
        (MAIN_HOST, "/", "no-cache"),
        (MAIN_HOST, stylesheet, "public, max-age=31536000, immutable"),
        (MAIN_HOST, "/admin", "no-store"),
        (MAIN_HOST, "/admin/login", "no-store"),
        (MAIN_HOST, "/does-not-exist", "no-store"),
        ("evil.example", "/", "no-store"),
        (MAIN_HOST, "/healthz", "no-store"),
        (
            MAIN_HOST,
            "/.well-known/kohaku/tls-ask?domain=a.test",
            "no-store",
        ),
    ];
    for (host, path, class) in expect {
        let response = harness.get(host, path).await;
        assert_eq!(
            header_values(&response, "cache-control"),
            [class],
            "{host} {path}"
        );
    }
    let mut stale = stylesheet.to_owned();
    stale.replace_range(15..16, if &stale[15..16] == "0" { "1" } else { "0" });
    let response = harness.get(MAIN_HOST, &stale).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(header_values(&response, "cache-control"), ["no-cache"]);
}

#[tokio::test]
async fn landing_page_markup_is_csp_clean() {
    let harness = Harness::new();
    for (host, path) in [
        (MAIN_HOST, "/"),
        (MAIN_HOST, "/nothing"),
        ("evil.example", "/"),
    ] {
        let html = body_text(harness.get(host, path).await).await;
        let lower = html.to_lowercase();
        assert!(!lower.contains("<script"), "{path}");
        assert!(!lower.contains("<style"), "{path}");
        assert!(!lower.contains("style="), "{path}");
        assert!(!lower.contains(" on"), "{path}: event-handler attribute");
        assert!(!lower.contains("data:"), "{path}");
        // Only hashed /static/ resources and the one absolute footer link.
        let links: Vec<&str> = html
            .split(['"', '\''])
            .filter(|part| part.starts_with('/') || part.starts_with("http"))
            .collect();
        for link in &links {
            assert!(
                link.starts_with("/static/kohaku.") || *link == "https://github.com/Dakes/Kohaku",
                "{path}: {link}"
            );
        }
        assert!(links.contains(&assets::path("kohaku.css")));
    }
}
