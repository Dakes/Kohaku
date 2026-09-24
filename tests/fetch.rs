//! Fetch-Metadata/Origin check, admin isolation, body cap, multipart and deadline
//! (http-security; change foundation D14).

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::http::StatusCode;
use axum::routing::post;
use kohaku::routing::hosts::{HostMap, ProjectId};
use kohaku::routing::table::{Headerless, HostKind, table};
use support::*;

const PROJECT_HOST: &str = "bugs.example.net";
const MAIN_ORIGIN: &str = "https://kohaku.example.org";

fn counting_route(
    pattern: &'static str,
    calls: &Arc<AtomicUsize>,
    exempt: bool,
) -> kohaku::routing::table::Route {
    let calls = Arc::clone(calls);
    let mut route = synthetic_route(
        HostKind::Both,
        pattern,
        post(move |body: Bytes| {
            let calls = Arc::clone(&calls);
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                format!("{}", body.len())
            }
        }),
    );
    if exempt {
        route.headerless = Headerless::Exempt;
    }
    route
}

fn harness_with(calls: &Arc<AtomicUsize>, base_url: &str) -> Harness {
    let mut routes = table();
    routes.push(counting_route("/write", calls, false));
    routes.push(counting_route("/api/write", calls, true));
    let harness = Harness::with(routes, &[("KOHAKU_BASE_URL", Some(base_url))]);
    harness
        .app
        .host_map
        .replace(HostMap::new(&harness.app.base_url).with_project(PROJECT_HOST, ProjectId(1)));
    harness
}

async fn post_status(
    harness: &Harness,
    host: &str,
    path: &str,
    headers: &[(&str, &str)],
) -> StatusCode {
    let mut builder = request("POST", host, path);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let response = harness.send(builder.body(Body::empty()).unwrap()).await;
    assert!(
        !response
            .headers()
            .keys()
            .any(|k| k.as_str().starts_with("access-control-allow-"))
    );
    response.status()
}

#[tokio::test]
async fn same_origin_writes_pass_and_internal_endpoints_are_outside() {
    let calls = Arc::new(AtomicUsize::new(0));
    let harness = harness_with(&calls, MAIN_ORIGIN);
    assert_eq!(
        post_status(&harness, MAIN_HOST, "/", &[("origin", MAIN_ORIGIN)]).await,
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        post_status(
            &harness,
            MAIN_HOST,
            "/",
            &[("origin", MAIN_ORIGIN), ("sec-fetch-site", "same-origin")]
        )
        .await,
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        post_status(
            &harness,
            MAIN_HOST,
            "/healthz",
            &[("sec-fetch-site", "cross-site")]
        )
        .await,
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        post_status(&harness, MAIN_HOST, "/.well-known/kohaku/tls-ask", &[]).await,
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        post_status(&harness, MAIN_HOST, "/write", &[("origin", MAIN_ORIGIN)]).await,
        StatusCode::OK
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cross_site_writes_are_refused_before_any_handler() {
    let calls = Arc::new(AtomicUsize::new(0));
    let harness = harness_with(&calls, MAIN_ORIGIN);
    for site in ["cross-site", "same-site", "none"] {
        for path in ["/", "/write"] {
            assert_eq!(
                post_status(&harness, MAIN_HOST, path, &[("sec-fetch-site", site)]).await,
                StatusCode::FORBIDDEN
            );
            assert_eq!(
                post_status(
                    &harness,
                    MAIN_HOST,
                    path,
                    &[("sec-fetch-site", site), ("origin", MAIN_ORIGIN)]
                )
                .await,
                StatusCode::FORBIDDEN
            );
        }
    }
    let twice = [
        [("origin", MAIN_ORIGIN), ("origin", "https://evil.example")],
        [
            ("sec-fetch-site", "same-origin"),
            ("sec-fetch-site", "cross-site"),
        ],
        [("origin", MAIN_ORIGIN), ("origin", MAIN_ORIGIN)],
    ];
    for headers in &twice {
        assert_eq!(
            post_status(&harness, MAIN_HOST, "/write", headers).await,
            StatusCode::FORBIDDEN
        );
    }
    for method in ["PUT", "PATCH", "DELETE", "PROPFIND", "OPTIONS"] {
        let response = harness
            .send(
                request(method, MAIN_HOST, "/write")
                    .header("sec-fetch-site", "cross-site")
                    .header("access-control-request-method", "POST")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method}");
        assert!(
            !response
                .headers()
                .keys()
                .any(|k| k.as_str().starts_with("access-control-allow-"))
        );
    }
    // A declared body that never arrives: answered without waiting for it.
    let started = Instant::now();
    assert_eq!(
        post_status(
            &harness,
            MAIN_HOST,
            "/write",
            &[
                ("sec-fetch-site", "cross-site"),
                ("content-length", "60000")
            ]
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    for (method, path) in [
        ("POST", "/"),
        ("PUT", "/admin/anything"),
        ("POST", "/does-not-exist"),
        ("POST", "/write"),
    ] {
        let response = harness
            .send(
                request(method, MAIN_HOST, path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} {path}");
    }
    // An exempt route still refuses null and cross-site.
    assert_eq!(
        post_status(&harness, MAIN_HOST, "/api/write", &[("origin", "null")]).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post_status(
            &harness,
            MAIN_HOST,
            "/api/write",
            &[("sec-fetch-site", "cross-site")]
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post_status(&harness, MAIN_HOST, "/api/write", &[]).await,
        StatusCode::OK
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        harness
            .app
            .counters
            .take()
            .iter()
            .any(|(r, n)| *r == kohaku::logging::Reason::CrossSite && *n > 0)
    );
}

#[tokio::test]
async fn null_near_miss_or_spoofed_origins() {
    let calls = Arc::new(AtomicUsize::new(0));
    let harness = harness_with(&calls, MAIN_ORIGIN);
    let origins = [
        "null",
        "https://evil.example",
        "http://kohaku.example.org",
        "https://kohaku.example.org:443",
        "https://kohaku.example.org/",
        "https://KOHAKU.EXAMPLE.ORG",
        "https://kohaku.example.org.evil.example",
    ];
    for origin in origins {
        for site in [None, Some("same-origin")] {
            let mut headers = vec![("origin", origin)];
            if let Some(site) = site {
                headers.push(("sec-fetch-site", site));
            }
            assert_eq!(
                post_status(&harness, MAIN_HOST, "/write", &headers).await,
                StatusCode::FORBIDDEN,
                "{origin} {site:?}"
            );
        }
    }
    let forged = [
        ("origin", "https://evil.example"),
        ("x-forwarded-host", "evil.example"),
        ("x-forwarded-proto", "http"),
        ("forwarded", "host=evil.example;proto=http"),
    ];
    assert_eq!(
        post_status(&harness, MAIN_HOST, "/write", &forged).await,
        StatusCode::FORBIDDEN
    );
    let upper = "KOHAKU.EXAMPLE.ORG:8080";
    assert_eq!(
        post_status(
            &harness,
            upper,
            "/write",
            &[("origin", "https://KOHAKU.EXAMPLE.ORG:8080")]
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post_status(&harness, upper, "/write", &[("origin", MAIN_ORIGIN)]).await,
        StatusCode::OK
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn writes_across_configured_hosts() {
    let calls = Arc::new(AtomicUsize::new(0));
    let harness = harness_with(&calls, "https://kohaku.example.org:8443");
    let main = "https://kohaku.example.org:8443";
    let project = "https://bugs.example.net";
    assert_eq!(
        post_status(
            &harness,
            MAIN_HOST,
            "/write",
            &[("origin", "https://kohaku.example.org")]
        )
        .await,
        StatusCode::FORBIDDEN
    );
    for site in [
        &[("sec-fetch-site", "same-site"), ("origin", project)][..],
        &[("origin", project)],
    ] {
        assert_eq!(
            post_status(&harness, MAIN_HOST, "/write", site).await,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        post_status(&harness, PROJECT_HOST, "/write", &[("origin", main)]).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post_status(&harness, MAIN_HOST, "/write", &[("origin", main)]).await,
        StatusCode::OK
    );
    assert_eq!(
        post_status(&harness, PROJECT_HOST, "/write", &[("origin", project)]).await,
        StatusCode::OK
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn admin_requests_are_isolated() {
    let harness = Harness::new();
    let get = |pairs: &'static [(&'static str, &'static str)], path: &'static str| {
        let harness = &harness;
        async move {
            let mut builder = request("GET", MAIN_HOST, path);
            for (name, value) in pairs {
                builder = builder.header(*name, *value);
            }
            harness
                .send(builder.body(Body::empty()).unwrap())
                .await
                .status()
        }
    };
    let refused: [&[(&str, &str)]; 4] = [
        &[
            ("sec-fetch-site", "cross-site"),
            ("sec-fetch-mode", "no-cors"),
        ],
        &[("sec-fetch-site", "same-site"), ("sec-fetch-mode", "cors")],
        &[("sec-fetch-site", "evil")],
        &[
            ("sec-fetch-site", "same-origin"),
            ("sec-fetch-site", "cross-site"),
        ],
    ];
    for headers in refused {
        assert_eq!(
            get(headers, "/admin/anything").await,
            StatusCode::FORBIDDEN,
            "{headers:?}"
        );
        assert_eq!(
            get(headers, "/admin").await,
            StatusCode::FORBIDDEN,
            "{headers:?}"
        );
    }
    let passed: [&[(&str, &str)]; 4] = [
        &[("sec-fetch-site", "same-origin")],
        &[("sec-fetch-site", "none")],
        &[],
        &[
            ("sec-fetch-site", "cross-site"),
            ("sec-fetch-mode", "navigate"),
        ],
    ];
    // Passed on to the session guard, which sends a request without a session to login.
    for headers in passed {
        assert_eq!(
            get(headers, "/admin").await,
            StatusCode::SEE_OTHER,
            "{headers:?}"
        );
    }
    // A refused request takes no read token.
    for _ in 0..400 {
        assert_eq!(get(refused[0], "/admin").await, StatusCode::FORBIDDEN);
    }
    assert_eq!(get(&[], "/").await, StatusCode::OK);
}

#[tokio::test]
async fn oversize_bodies() {
    let calls = Arc::new(AtomicUsize::new(0));
    let harness = harness_with(&calls, MAIN_ORIGIN);
    for path in ["/", "/write"] {
        let started = Instant::now();
        let status = post_status(
            &harness,
            MAIN_HOST,
            path,
            &[("origin", MAIN_ORIGIN), ("content-length", "65537")],
        )
        .await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{path}");
        assert!(started.elapsed() < Duration::from_secs(1));
    }
    let exact = harness
        .send(
            request("POST", MAIN_HOST, "/write")
                .header("origin", MAIN_ORIGIN)
                .body(Body::from(vec![b'x'; 65_536]))
                .unwrap(),
        )
        .await;
    assert_eq!(exact.status(), StatusCode::OK);
    assert_eq!(body_text(exact).await, "65536");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chunked_gigabyte_is_cut_off_at_the_cap() {
    use std::io::{Read, Write};
    let calls = Arc::new(AtomicUsize::new(0));
    let server = Server::start(harness_with(&calls, MAIN_ORIGIN)).await;
    let result = tokio::task::spawn_blocking(move || {
        let mut stream = server.connect();
        let head = format!(
            "POST /write HTTP/1.1\r\nHost: {MAIN_HOST}\r\nOrigin: {MAIN_ORIGIN}\r\nTransfer-Encoding: chunked\r\n\r\n"
        );
        stream.write_all(head.as_bytes()).unwrap();
        let chunk = format!("2000\r\n{}\r\n", "x".repeat(8192));
        let mut sent = 0usize;
        // 1 GiB unless the server stops reading first.
        while sent < (1 << 30) {
            if stream.write_all(chunk.as_bytes()).is_err() {
                break;
            }
            sent += 8192;
            if sent.is_multiple_of(1 << 20) {
                stream.set_nonblocking(true).unwrap();
                let mut peek = [0u8; 12];
                let got = stream.peek(&mut peek).unwrap_or(0);
                stream.set_nonblocking(false).unwrap();
                if got > 0 {
                    break;
                }
            }
        }
        let mut response = Vec::new();
        let _ = stream.read_to_end(&mut response);
        (sent, String::from_utf8_lossy(&response).into_owned(), server)
    })
    .await
    .unwrap();
    let (sent, response, _server) = result;
    assert!(response.starts_with("HTTP/1.1 413"), "{response}");
    assert!(
        sent < 64 << 20,
        "the server stopped reading long before 1 GiB ({sent} bytes)"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn multipart_is_refused_unread() {
    let calls = Arc::new(AtomicUsize::new(0));
    let harness = harness_with(&calls, MAIN_ORIGIN);
    let started = Instant::now();
    let declared = post_status(
        &harness,
        MAIN_HOST,
        "/",
        &[
            ("origin", MAIN_ORIGIN),
            ("content-type", "multipart/form-data; boundary=x"),
            ("content-length", "26214400"),
        ],
    )
    .await;
    assert_eq!(declared, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert!(started.elapsed() < Duration::from_secs(1));
    for media in [
        "Multipart/Form-Data",
        "MULTIPART/MIXED",
        "multipart/related",
    ] {
        assert_eq!(
            post_status(
                &harness,
                MAIN_HOST,
                "/write",
                &[("origin", MAIN_ORIGIN), ("content-type", media)]
            )
            .await,
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
    }
    let get = harness
        .send(
            request("GET", MAIN_HOST, "/")
                .header("content-type", "multipart/form-data")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(get.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
