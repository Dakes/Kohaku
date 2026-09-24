//! Rate limits, client addresses, permits and the deadline on the wire
//! (request-limits; http-security: Request deadline; change foundation D16).

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use kohaku::assets;
use kohaku::limits::permits::{Bound, busy};
use kohaku::limits::rate::RateClass;
use kohaku::logging::Reason;
use kohaku::routing::AppState;
use kohaku::routing::table::{HostKind, table};
use support::*;

const MAIN_ORIGIN: &str = "https://kohaku.example.org";

#[tokio::test]
async fn attacker_scans_unknown_paths() {
    let harness = Harness::new();
    for _ in 0..150 {
        let response = harness
            .send(request("HEAD", MAIN_HOST, "/").body(Body::empty()).unwrap())
            .await;
        assert_eq!(response.status(), StatusCode::OK);
    }
    for n in 0..150 {
        assert_eq!(
            harness
                .get(MAIN_HOST, &format!("/nothing-{n}"))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        harness.get(MAIN_HOST, "/").await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        harness
            .get(MAIN_HOST, assets::path("kohaku.css"))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        harness.app.counters.take(),
        [(Reason::RateLimited(RateClass::Read), 1)]
    );
}

#[tokio::test]
async fn multipart_get_takes_a_read_token() {
    let harness = Harness::new();
    let multipart = || {
        request("GET", MAIN_HOST, "/")
            .header("content-type", "multipart/form-data; boundary=x")
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        harness.send(multipart()).await.status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    for _ in 0..299 {
        assert_eq!(harness.get(MAIN_HOST, "/").await.status(), StatusCode::OK);
    }
    assert_eq!(
        harness.send(multipart()).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn internal_endpoints_draw_no_tokens() {
    let harness = Harness::with(table(), &[("KOHAKU_TRUSTED_PROXIES", Some("10.231.7.2"))]);
    for _ in 0..1000 {
        let healthz = harness
            .send(request("GET", "", "/healthz").body(Body::empty()).unwrap())
            .await;
        assert_eq!(healthz.status(), StatusCode::OK);
        let ask = from_peer(
            request(
                "GET",
                "kohaku:8080",
                "/.well-known/kohaku/tls-ask?domain=bugs.example.com",
            )
            .body(Body::empty())
            .unwrap(),
            "10.231.7.2:5000",
        );
        assert_eq!(harness.send(ask).await.status(), StatusCode::NOT_FOUND);
    }
    for _ in 0..300 {
        assert_eq!(harness.get(MAIN_HOST, "/").await.status(), StatusCode::OK);
    }
    assert!(harness.app.limiter.len() <= 1);
}

async fn counted(State(app): State<AppState>) -> &'static str {
    let _ = app;
    COUNTED.fetch_add(1, Ordering::SeqCst);
    "counted"
}

static COUNTED: AtomicUsize = AtomicUsize::new(0);

#[tokio::test]
async fn rate_limited_requests_do_no_work() {
    let mut route = synthetic_route(HostKind::Main, "/counted", get(counted));
    route.rate = &[RateClass::Read];
    let mut routes = table();
    routes.push(route);
    let harness = Harness::with(routes, &[]);
    for _ in 0..300 {
        assert_eq!(
            harness.get(MAIN_HOST, "/counted").await.status(),
            StatusCode::OK
        );
    }
    let before = COUNTED.load(Ordering::SeqCst);
    for _ in 0..1000 {
        let response = harness.get(MAIN_HOST, "/counted").await;
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        let headers = format!("{:?}", response.headers());
        assert!(!headers.contains("203.0.113.5"));
        assert!(!body_text(response).await.contains("203.0.113.5"));
    }
    assert_eq!(COUNTED.load(Ordering::SeqCst), before);
}

#[tokio::test]
async fn forwarded_addresses_key_the_buckets() {
    let harness = Harness::with(table(), &[("KOHAKU_TRUSTED_PROXIES", Some("10.231.7.2"))]);
    let via_proxy = |client: &str| {
        from_peer(
            request("GET", MAIN_HOST, "/")
                .header("x-forwarded-for", client)
                .body(Body::empty())
                .unwrap(),
            "[::ffff:10.231.7.2]:5000",
        )
    };
    for _ in 0..300 {
        assert_eq!(
            harness.send(via_proxy("198.51.100.7")).await.status(),
            StatusCode::OK
        );
    }
    assert_eq!(
        harness.send(via_proxy("198.51.100.7")).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        harness.send(via_proxy("198.51.100.8")).await.status(),
        StatusCode::OK
    );
    // An untrusted peer's header is ignored: its own address is the key.
    let forged = from_peer(
        request("GET", MAIN_HOST, "/")
            .header("x-forwarded-for", "198.51.100.9")
            .body(Body::empty())
            .unwrap(),
        "203.0.113.9:1",
    );
    assert_eq!(harness.send(forged).await.status(), StatusCode::OK);
}

/// Takes a public write permit and holds it until `release` is notified.
async fn held_write(State(app): State<AppState>) -> Response {
    let Some(permit) = app.permits.try_public_write() else {
        return busy(Bound::PublicWrites);
    };
    STARTED.fetch_add(1, Ordering::SeqCst);
    RELEASE.notified().await;
    drop(permit);
    "written".into_response()
}

static STARTED: AtomicUsize = AtomicUsize::new(0);
static RELEASE: tokio::sync::Notify = tokio::sync::Notify::const_new();

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn attacker_floods_public_writes() {
    let mut routes = table();
    routes.push(synthetic_route(HostKind::Main, "/held", post(held_write)));
    let harness = Arc::new(Harness::with(routes, &[]));
    let send = |harness: Arc<Harness>| async move {
        let started = Instant::now();
        let response = harness
            .send(
                request("POST", MAIN_HOST, "/held")
                    .header("origin", MAIN_ORIGIN)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        (response.status(), started.elapsed())
    };
    let tasks: Vec<_> = (0..20)
        .map(|_| tokio::spawn(send(Arc::clone(&harness))))
        .collect();
    while STARTED.load(Ordering::SeqCst) < 8 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(STARTED.load(Ordering::SeqCst), 8);
    RELEASE.notify_waiters();
    let mut busy_count = 0;
    for task in tasks {
        let (status, elapsed) = task.await.unwrap();
        if status == StatusCode::SERVICE_UNAVAILABLE {
            busy_count += 1;
            assert!(elapsed < Duration::from_millis(100), "503 at once");
        } else {
            assert_eq!(status, StatusCode::OK);
        }
    }
    assert_eq!(busy_count, 12);
    assert_eq!(
        harness.app.counters.take(),
        [(Reason::Busy(Bound::PublicWrites), 12)]
    );
    assert!(harness.app.permits.try_public_write().is_some());
}

/// Takes a permit, then reads the body (a trickled one never completes in time).
async fn permit_then_body(State(app): State<AppState>, body: Bytes) -> Response {
    let _ = body;
    match app.permits.try_public_write() {
        Some(_permit) => "ok".into_response(),
        None => busy(Bound::PublicWrites),
    }
}

async fn permit_then_sleep(State(app): State<AppState>) -> Response {
    let Some(_permit) = app.permits.try_public_write() else {
        return busy(Bound::PublicWrites);
    };
    tokio::time::sleep(Duration::from_secs(20)).await;
    "late".into_response()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn permit_is_released_at_the_deadline() {
    let mut routes = table();
    routes.push(synthetic_route(
        HostKind::Main,
        "/trickle",
        post(permit_then_body),
    ));
    routes.push(synthetic_route(
        HostKind::Main,
        "/sleep",
        post(permit_then_sleep),
    ));
    let server = Server::start(Harness::with(routes, &[])).await;
    let addr = server.addr;
    let trickle = tokio::spawn(async move {
        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let head = format!(
            "POST /trickle HTTP/1.1\r\nHost: {MAIN_HOST}\r\nOrigin: {MAIN_ORIGIN}\r\nContent-Length: 60000\r\n\r\n"
        );
        write_all(&stream, head.as_bytes()).await.unwrap();
        let started = Instant::now();
        let mut response = vec![0u8; 12];
        loop {
            tokio::select! {
                read = read_some(&stream, &mut response) => {
                    let n = read.unwrap();
                    return (String::from_utf8_lossy(&response[..n]).into_owned(), started.elapsed());
                }
                () = tokio::time::sleep(Duration::from_secs(1)) => {
                    let _ = write_all(&stream, b"x").await;
                }
            }
        }
    });
    let started = Instant::now();
    let sleep_status = wire_request(
        addr,
        "POST",
        MAIN_HOST,
        "/sleep",
        &[("origin", MAIN_ORIGIN), ("content-length", "0")],
    )
    .await;
    let sleep_elapsed = started.elapsed();
    let (trickle_response, trickle_elapsed) = trickle.await.unwrap();
    assert_eq!(sleep_status, Some(408));
    assert!(
        trickle_response.starts_with("HTTP/1.1 408"),
        "{trickle_response}"
    );
    for elapsed in [sleep_elapsed, trickle_elapsed] {
        assert!(
            elapsed >= Duration::from_secs(15) && elapsed < Duration::from_secs(17),
            "{elapsed:?}"
        );
    }
    // Every permit is free again.
    let permits: Vec<_> = (0..8)
        .map(|_| server.harness.app.permits.try_public_write())
        .collect();
    assert!(permits.iter().all(Option::is_some));
}

#[tokio::test(flavor = "current_thread")]
async fn warnings_are_logged_once_without_addresses() {
    let capture = LogCapture::default();
    let _guard = capture.install();
    let server = Server::start(Harness::with(
        table(),
        &[("KOHAKU_TRUSTED_PROXIES", Some("127.0.0.1"))],
    ))
    .await;
    let addr = server.addr;
    // healthcheck and the TLS ask resolve no address and warn about nothing.
    assert_eq!(
        wire_request(addr, "GET", "127.0.0.1:8080", "/healthz", &[]).await,
        Some(200)
    );
    wire_request(
        addr,
        "GET",
        "kohaku:8080",
        "/.well-known/kohaku/tls-ask?domain=a.test",
        &[],
    )
    .await;
    assert!(!capture.text().contains("WARN"), "{}", capture.text());
    // The trusted peer is 127.0.0.1 itself here.
    let xff = |value: &'static str| [("x-forwarded-for", value)];
    assert_eq!(
        wire_request(addr, "GET", MAIN_HOST, "/", &xff("198.51.100.7")).await,
        Some(200)
    );
    let quiet = capture.text();
    assert!(!quiet.contains("WARN"), "{quiet}");
    for _ in 0..2 {
        wire_request(addr, "GET", MAIN_HOST, "/", &xff("172.18.0.1")).await;
    }
    for _ in 0..2 {
        wire_request(addr, "GET", MAIN_HOST, "/", &[]).await;
    }
    let log = capture.text();
    assert_eq!(
        log.matches("client address is loopback/RFC 1918/ULA/link-local")
            .count(),
        1,
        "{log}"
    );
    assert_eq!(log.matches("trusted peer sent no XFF").count(), 1, "{log}");
    for address in ["198.51.100.7", "172.18.0.1", "127.0.0.1"] {
        assert!(!log.contains(address), "{log}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn forged_header_from_untrusted_peer_warns_once() {
    let capture = LogCapture::default();
    let _guard = capture.install();
    let server = Server::start(Harness::with(
        table(),
        &[("KOHAKU_TRUSTED_PROXIES", Some("10.231.7.2"))],
    ))
    .await;
    for _ in 0..1000 {
        wire_request(
            server.addr,
            "GET",
            MAIN_HOST,
            "/healthz",
            &[("x-forwarded-for", "192.0.2.1")],
        )
        .await;
    }
    for _ in 0..50 {
        wire_request(
            server.addr,
            "GET",
            MAIN_HOST,
            "/",
            &[("x-forwarded-for", "192.0.2.1")],
        )
        .await;
    }
    let log = capture.text();
    assert_eq!(log.matches("XFF from untrusted peer").count(), 1, "{log}");
    assert!(
        !log.contains("192.0.2.1") && !log.contains("127.0.0.1"),
        "{log}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn attacker_floods_the_health_endpoint() {
    let capture = LogCapture::default();
    let _guard = capture.install();
    let harness = Arc::new(Harness::new());
    let permits: Vec<_> = (0..8)
        .map(|_| harness.app.permits.try_public_write().unwrap())
        .collect();
    // Hold the writer and every reader.
    let (hold, held) = std::sync::mpsc::channel::<()>();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let db = Arc::clone(&harness.app.db);
    let writer = tokio::spawn(async move {
        db.write(move |_| {
            hold.send(()).unwrap();
            released.recv().unwrap();
            Ok::<_, rusqlite::Error>(())
        })
        .await
    });
    tokio::task::spawn_blocking(move || held.recv().unwrap())
        .await
        .unwrap();
    for _ in 0..500 {
        let started = Instant::now();
        let response = harness
            .send(request("GET", "", "/healthz").body(Body::empty()).unwrap())
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(started.elapsed() < Duration::from_millis(100));
    }
    release.send(()).unwrap();
    writer.await.unwrap().unwrap();
    drop(permits);
    assert!(capture.text().is_empty(), "{}", capture.text());
}
