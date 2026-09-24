//! `serve` as a whole: startup order, lock, listening, shutdown and what the data
//! directory and logs may hold (operations; data-storage; configuration).

mod support;

use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use kohaku::db::backup::{backup_to_file, list_backups, restore};
use kohaku::db::lock::InstanceLock;
use kohaku::db::migrate::{MIGRATIONS, MigrateError, open_and_prepare};
use kohaku::db::{DataDir, Role, open};
use kohaku::keys::PerBootKey;
use kohaku::mail::{Mailer, Outgoing, SendFuture};
use kohaku::serve::{Listen, MailerChoice, ServeError, serve};
use support::*;
use tokio::sync::oneshot;

/// Accepts every message.
struct NullMailer;

impl Mailer for NullMailer {
    fn send(&self, _: Outgoing) -> SendFuture<'_> {
        Box::pin(async { Ok(()) })
    }
}

struct Running {
    addr: SocketAddr,
    stop: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<Result<(), ServeError>>,
}

impl Running {
    async fn stop(mut self) -> Result<(), ServeError> {
        let _ = self.stop.take().unwrap().send(());
        self.task.await.unwrap()
    }
}

/// Starts `serve` on 127.0.0.1 with `changes` to the valid environment.
async fn start(
    data: &DataDir,
    changes: &[(&str, Option<&str>)],
    mailer: MailerChoice,
) -> Result<Running, ServeError> {
    let config = serve_config(changes);
    let (stop, stopped) = oneshot::channel::<()>();
    let (listening, bound) = oneshot::channel();
    let data = data.clone();
    let task = tokio::spawn(async move {
        serve(
            config,
            data,
            Listen::Address("127.0.0.1:0".parse().unwrap()),
            mailer,
            move |addr| {
                let _ = listening.send(addr);
            },
            async move {
                let _ = stopped.await;
            },
        )
        .await
    });
    match bound.await {
        Ok(addr) => Ok(Running {
            addr,
            stop: Some(stop),
            task,
        }),
        Err(_) => Err(task.await.unwrap().unwrap_err()),
    }
}

fn null() -> MailerChoice {
    MailerChoice::Given(Arc::new(NullMailer))
}

async fn healthz(addr: SocketAddr) -> Option<u16> {
    wire_request(addr, "GET", "x", "/healthz", &[]).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wrong_secret_refused() {
    let (dir, data) = data_dir();
    let first = start(&data, &[], null()).await.unwrap();
    assert_eq!(healthz(first.addr).await, Some(200));
    first.stop().await.unwrap();
    let again = start(&data, &[], null()).await.unwrap();
    assert_eq!(healthz(again.addr).await, Some(200));
    again.stop().await.unwrap();
    let before = fs::read(data.database()).unwrap();
    let error = start(&data, &[("KOHAKU_SECRET", Some(OTHER_TEST_SECRET))], null())
        .await
        .err()
        .unwrap();
    assert!(matches!(
        error,
        ServeError::Database(MigrateError::KeycheckMismatch)
    ));
    assert_eq!(
        error.to_string(),
        "KOHAKU_SECRET does not match the database"
    );
    assert_eq!(fs::read(data.database()).unwrap(), before);
    assert!(!data.backups().exists());
    drop(dir);
}

#[cfg(not(feature = "dev"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unreachable_smtp_server_does_not_block_startup() {
    let (_dir, data) = data_dir();
    let running = start(
        &data,
        &[
            ("KOHAKU_SMTP_HOST", Some("smtp.kohaku.test")),
            ("KOHAKU_SMTP_PORT", Some("587")),
            ("KOHAKU_SMTP_TLS", Some("starttls")),
            ("KOHAKU_SMTP_USERNAME", Some("kohaku")),
            ("KOHAKU_SMTP_PASSWORD", Some("correct horse battery staple")),
            ("KOHAKU_SMTP_FROM", Some("kohaku@bugs.kohaku.test")),
        ],
        MailerChoice::Configured,
    )
    .await
    .unwrap();
    assert_eq!(healthz(running.addr).await, Some(200));
    running.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nothing_listens_before_startup_has_finished() {
    let (_dir, data) = data_dir();
    // Another process holds the database while a new one is being created.
    kohaku::db::create_file(&data.database()).unwrap();
    let blocker = rusqlite::Connection::open(data.database()).unwrap();
    blocker.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let port = {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        probe.local_addr().unwrap().port()
    };
    let config = serve_config(&[]);
    let (stop, stopped) = oneshot::channel::<()>();
    let started = Instant::now();
    let task = tokio::spawn(serve(
        config,
        data.clone(),
        Listen::Address(SocketAddr::from(([127, 0, 0, 1], port))),
        null(),
        |_| {},
        async move {
            let _ = stopped.await;
        },
    ));
    while started.elapsed() < Duration::from_millis(1500) {
        assert!(
            TcpStream::connect(("127.0.0.1", port)).is_err(),
            "connected during startup"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    blocker.execute_batch("COMMIT").unwrap();
    drop(blocker);
    let mut ready = false;
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ready);
    let _ = stop.send(());
    task.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_commands_while_serving() {
    let (dir, data) = data_dir();
    let running = start(&data, &[], null()).await.unwrap();
    let listing_before = {
        let mut names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        names.sort();
        names
    };
    let second = start(&data, &[], null()).await.err().unwrap();
    assert!(
        second
            .to_string()
            .contains("another Kohaku process is using /data")
    );
    let error = restore(&data, &mut std::io::empty(), 1).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("another Kohaku process is using /data")
    );
    let mut after: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    after.sort();
    assert_eq!(after, listing_before);
    let files = TempDir::new();
    backup_to_file(&data, &secret(), &files.path().join("b.db")).unwrap();
    list_backups(&data).unwrap();
    assert_eq!(healthz(running.addr).await, Some(200));
    running.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reachable_on_ipv4_and_ipv6_loopback() {
    let (_dir, data) = data_dir();
    let config = serve_config(&[]);
    let (stop, stopped) = oneshot::channel::<()>();
    let (listening, bound) = oneshot::channel();
    let task = tokio::spawn(serve(
        config,
        data.clone(),
        Listen::AllAddresses(0),
        null(),
        move |addr| {
            let _ = listening.send(addr);
        },
        async move {
            let _ = stopped.await;
        },
    ));
    let port = bound.await.unwrap().port();
    assert_eq!(
        wire_request(
            SocketAddr::from(([127, 0, 0, 1], port)),
            "GET",
            "x",
            "/healthz",
            &[]
        )
        .await,
        Some(200)
    );
    if std::net::TcpListener::bind("[::1]:0").is_ok() {
        let v6: SocketAddr = format!("[::1]:{port}").parse().unwrap();
        assert_eq!(
            wire_request(v6, "GET", "x", "/healthz", &[]).await,
            Some(200)
        );
    }
    let _ = stop.send(());
    task.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn idle_server_stops_at_once_and_cleanly() {
    let (_dir, data) = data_dir();
    let running = start(&data, &[], null()).await.unwrap();
    // One idle keep-alive connection.
    let idle = tokio::net::TcpStream::connect(running.addr).await.unwrap();
    write_all(&idle, b"GET /healthz HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    let mut first = Vec::new();
    let mut buf = [0u8; 512];
    while !first.ends_with(br#"{"status":"ok"}"#) {
        let n = read_some(&idle, &mut buf).await.unwrap();
        assert!(n > 0);
        first.extend_from_slice(&buf[..n]);
    }
    let started = Instant::now();
    running.stop().await.unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    let mut rest = [0u8; 16];
    assert_eq!(
        read_some(&idle, &mut rest).await.unwrap_or(0),
        0,
        "connection closed"
    );
    assert!(!data.wal().exists(), "no WAL left behind");
    restore(&data, &mut std::io::empty(), 1).unwrap_err();
    InstanceLock::acquire(&data).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn request_in_flight_completes_and_new_connections_are_refused() {
    let (_dir, data) = data_dir();
    let running = start(&data, &[], null()).await.unwrap();
    let addr = running.addr;
    let client = tokio::net::TcpStream::connect(addr).await.unwrap();
    write_all(&client, b"GET / HTTP/1.1\r\nHost: kohaku.example.org\r\n")
        .await
        .unwrap();
    let stopping = tokio::spawn(running.stop());
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        tokio::net::TcpStream::connect(addr).await.is_err() || {
            // Accepted by the kernel backlog before close, but never served.
            wire_request(addr, "GET", "kohaku.example.org", "/healthz", &[])
                .await
                .is_none()
        }
    );
    write_all(&client, b"\r\n").await.unwrap();
    let mut response = Vec::new();
    let mut buf = [0u8; 4096];
    while let Ok(n) = read_some(&client, &mut buf).await {
        if n == 0 {
            break;
        }
        response.extend_from_slice(&buf[..n]);
    }
    assert!(
        response.starts_with(b"HTTP/1.1 200"),
        "{}",
        String::from_utf8_lossy(&response)
    );
    stopping.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn attacker_holding_a_request_open_is_cut_off() {
    let (_dir, data) = data_dir();
    let running = start(&data, &[], null()).await.unwrap();
    let client = tokio::net::TcpStream::connect(running.addr).await.unwrap();
    write_all(
        &client,
        b"GET / HTTP/1.1\r\nHost: kohaku.example.org\r\nX-Slow: ",
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let stop_started = Instant::now();
    let stopping = tokio::spawn(running.stop());
    // One header byte per second until the server closes the connection.
    let closed_after = loop {
        let mut buf = [0u8; 64];
        match tokio::time::timeout(Duration::from_secs(1), read_some(&client, &mut buf)).await {
            Ok(Ok(0) | Err(_)) => break stop_started.elapsed(),
            Ok(Ok(_)) => panic!("the held request got a response"),
            Err(_) => {
                if write_all(&client, b"a").await.is_err() {
                    break stop_started.elapsed();
                }
            }
        }
        assert!(
            stop_started.elapsed() < Duration::from_secs(20),
            "never closed"
        );
    };
    stopping.await.unwrap().unwrap();
    let stopped_after = stop_started.elapsed();
    assert!(
        closed_after <= Duration::from_millis(8_600),
        "{closed_after:?}"
    );
    assert!(stopped_after < Duration::from_secs(10), "{stopped_after:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_migrating_start_records_no_audit_entry() {
    let (_dir, data) = data_dir();
    drop(open_and_prepare(&data, &secret(), &MIGRATIONS[..1], 1).unwrap());
    let running = start(&data, &[], null()).await.unwrap();
    running.stop().await.unwrap();
    let conn = open(&data.database(), Role::Command).unwrap();
    let n: i64 = conn
        .query_row("SELECT count(*) FROM audit_log", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn secrets_and_keys_never_reach_disk_or_logs() {
    let capture = LogCapture::default();
    let _guard = capture.install();
    let (dir, data) = data_dir();
    let keys = [
        PerBootKey::generate().unwrap(),
        PerBootKey::generate().unwrap(),
    ];
    let running = start(&data, &[], null()).await.unwrap();
    for n in 0..20 {
        wire_request(
            running.addr,
            "GET",
            "kohaku.example.org",
            &format!("/x{n}"),
            &[("x-forwarded-for", "198.51.100.7")],
        )
        .await;
    }
    let files = TempDir::new();
    backup_to_file(&data, &secret(), &files.path().join("b.db")).unwrap();
    let log_while_running = capture.text();
    running.stop().await.unwrap();
    let secret_bytes = b"kohaku-test-secret-32-bytes-long".to_vec();
    let mut needles: Vec<Vec<u8>> = vec![
        secret_bytes,
        TEST_SECRET.as_bytes().to_vec(),
        TEST_SMTP_PASSWORD.as_bytes().to_vec(),
        b"198.51.100.7".to_vec(),
    ];
    for key in &keys {
        let tag = key.mac(kohaku::keys::Purpose::Keycheck, &[]);
        needles.push(tag.to_vec());
    }
    let mut haystacks: Vec<(String, Vec<u8>)> =
        vec![("log".to_owned(), log_while_running.into_bytes())];
    for root in [dir.path(), files.path()] {
        let mut pending = vec![root.to_path_buf()];
        while let Some(path) = pending.pop() {
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                pending.extend(fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
            } else {
                haystacks.push((path.display().to_string(), fs::read(&path).unwrap()));
            }
        }
    }
    for (name, bytes) in &haystacks {
        for needle in &needles {
            assert!(
                !bytes.windows(needle.len()).any(|w| w == &needle[..]),
                "{name} holds a secret, key or address"
            );
        }
    }
}
