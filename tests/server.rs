//! The HTTP/1.1 server on real sockets (http-security: Request header read timeout;
//! change foundation D10).

mod support;

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use support::*;

/// Reads until the server closes; `(bytes, elapsed)`.
fn read_until_closed(stream: &mut std::net::TcpStream, started: Instant) -> (Vec<u8>, Duration) {
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);
    (response, started.elapsed())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stalled_headers_are_closed_without_a_response() {
    let server = Server::start(Harness::new()).await;
    let addr = server.addr;
    let stalled = tokio::task::spawn_blocking(move || {
        let mut stream = std::net::TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        let started = Instant::now();
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: kohaku.example.org\r\n")
            .unwrap();
        read_until_closed(&mut stream, started)
    });
    let trickling: Vec<_> = (0..100)
        .map(|_| {
            tokio::task::spawn_blocking(move || {
                let mut stream = std::net::TcpStream::connect(addr).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let started = Instant::now();
                let head =
                    b"GET / HTTP/1.1\r\nHost: kohaku.example.org\r\nX-Slow: aaaaaaaaaaaaaaaaaaaa";
                let mut response = Vec::new();
                for byte in head {
                    if stream.write_all(&[*byte]).is_err() {
                        break;
                    }
                    let mut buf = [0u8; 256];
                    match stream.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => response.extend_from_slice(&buf[..n]),
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                        Err(_) => break,
                    }
                }
                (response, started.elapsed())
            })
        })
        .collect();
    let (response, elapsed) = stalled.await.unwrap();
    assert!(
        response.is_empty(),
        "{}",
        String::from_utf8_lossy(&response)
    );
    assert!(
        elapsed >= Duration::from_secs(9) && elapsed < Duration::from_secs(11),
        "{elapsed:?}"
    );
    for task in trickling {
        let (response, elapsed) = task.await.unwrap();
        assert!(response.is_empty());
        assert!(elapsed < Duration::from_secs(13), "{elapsed:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn header_timeout_applies_to_kept_alive_requests() {
    let server = Server::start(Harness::new()).await;
    let addr = server.addr;
    let (first, rest, elapsed) = tokio::task::spawn_blocking(move || {
        let mut stream = std::net::TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        stream
            .write_all(b"GET /healthz HTTP/1.1\r\nHost: x\r\n\r\n")
            .unwrap();
        let mut first = vec![0u8; 4096];
        let n = stream.read(&mut first).unwrap();
        first.truncate(n);
        let started = Instant::now();
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: kohaku.example.org\r\n")
            .unwrap();
        let (rest, elapsed) = read_until_closed(&mut stream, started);
        (first, rest, elapsed)
    })
    .await
    .unwrap();
    assert!(String::from_utf8_lossy(&first).starts_with("HTTP/1.1 200"));
    assert!(rest.is_empty(), "{}", String::from_utf8_lossy(&rest));
    assert!(
        elapsed >= Duration::from_secs(9) && elapsed < Duration::from_secs(11),
        "{elapsed:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn raw_markup_in_the_request_line_is_not_reflected() {
    let server = Server::start(Harness::new()).await;
    let addr = server.addr;
    let response = tokio::task::spawn_blocking(move || {
        raw_exchange(
            addr,
            b"GET /?q=<script>alert(1)</script> HTTP/1.1\r\nHost: kohaku.example.org\r\nConnection: close\r\n\r\n",
        )
    })
    .await
    .unwrap();
    assert!(!response.contains("<script>alert"), "{response}");
}

#[test]
fn no_http2_in_the_dependency_tree() {
    let lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock")).unwrap();
    assert!(!lock.contains("name = \"h2\""));
}
