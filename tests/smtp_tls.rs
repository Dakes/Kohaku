//! The SMTP adapter against a local peer: verified TLS only, nothing before it
//! (mail-outbox: SMTP delivery requires verified TLS, SMTP credentials and mail data
//! stay out of logs and errors; change foundation D18).

mod support;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread::JoinHandle;

use kohaku::config::{DnsName, SmtpConfig, load_smtp};
use kohaku::mail::smtp::{SmtpMailer, SmtpRoots};
use kohaku::mail::{FailureClass, Mailer, Outgoing, build_message};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use support::*;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/smtp/");
const USERNAME: &str = "kohaku-user";
const PASSWORD: &str = "correct horse battery staple";

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{FIXTURES}{name}")).unwrap()
}

fn server_config(leaf: &str) -> Arc<ServerConfig> {
    let certs = vec![CertificateDer::from_pem_slice(&fixture(&format!("{leaf}.pem"))).unwrap()];
    let key = PrivateKeyDer::from_pem_slice(&fixture(&format!("{leaf}.key"))).unwrap();
    let config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .unwrap();
    Arc::new(config)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Peer {
    /// TLS from the first byte.
    Implicit(&'static str),
    /// Offers STARTTLS, then TLS with this leaf.
    Starttls(&'static str),
    /// Never offers STARTTLS (stripped by an attacker).
    Stripped,
    /// Accepts STARTTLS, then answers garbage instead of a handshake.
    BrokenHandshake,
    /// Implicit TLS, then rejects AUTH with 535.
    RejectsAuth,
    /// Implicit TLS, then rejects the recipient, echoing it.
    RejectsRecipient,
}

/// Everything the peer received, as text.
struct Transcript {
    received: String,
}

impl Transcript {
    fn saw_nothing_after_tls(&self) {
        for command in ["AUTH", "MAIL FROM", "RCPT TO", "DATA"] {
            assert!(
                !self.received.contains(command),
                "{command} was sent: {}",
                self.received
            );
        }
        assert!(!self.received.contains(PASSWORD));
        use base64::Engine as _;
        let plain =
            base64::engine::general_purpose::STANDARD.encode(format!("\0{USERNAME}\0{PASSWORD}"));
        assert!(!self.received.contains(&plain));
    }
}

trait Stream: Read + Write {}
impl<T: Read + Write> Stream for T {}

fn read_line(stream: &mut dyn Stream, received: &mut String) -> Option<String> {
    let mut line = Vec::new();
    let mut byte = [0u8];
    loop {
        match stream.read(&mut byte) {
            Ok(1) => {
                line.push(byte[0]);
                if line.ends_with(b"\r\n") {
                    let text = String::from_utf8_lossy(&line).into_owned();
                    received.push_str(&text);
                    return Some(text);
                }
            }
            _ => return None,
        }
    }
}

fn say(stream: &mut dyn Stream, text: &str) -> Option<()> {
    stream.write_all(text.as_bytes()).ok()?;
    stream.flush().ok()
}

/// Serves one SMTP session on `stream`; returns when the client quits or fails.
fn session(
    stream: &mut dyn Stream,
    peer: Peer,
    tls_offered: bool,
    received: &mut String,
) -> Option<TcpStream> {
    loop {
        let line = read_line(stream, received)?;
        let command = line.to_ascii_uppercase();
        if command.starts_with("EHLO") {
            let starttls = if tls_offered { "250-STARTTLS\r\n" } else { "" };
            say(
                stream,
                &format!("250-localhost\r\n{starttls}250 AUTH PLAIN\r\n"),
            )?;
        } else if command.starts_with("STARTTLS") {
            say(stream, "220 go ahead\r\n")?;
            return None;
        } else if command.starts_with("AUTH") {
            if peer == Peer::RejectsAuth {
                say(stream, "535 5.7.8 Authentication credentials invalid\r\n")?;
            } else {
                say(stream, "235 ok\r\n")?;
            }
        } else if command.starts_with("MAIL FROM") {
            say(stream, "250 ok\r\n")?;
        } else if command.starts_with("RCPT TO") {
            if peer == Peer::RejectsRecipient {
                say(
                    stream,
                    "550 5.1.1 <victim@example.com>: Recipient address rejected\r\n",
                )?;
            } else {
                say(stream, "250 ok\r\n")?;
            }
        } else if command.starts_with("DATA") {
            say(stream, "354 go on\r\n")?;
            loop {
                let data = read_line(stream, received)?;
                if data == ".\r\n" {
                    break;
                }
            }
            say(stream, "250 queued\r\n")?;
        } else if command.starts_with("QUIT") {
            say(stream, "221 bye\r\n")?;
            return None;
        } else {
            say(stream, "500 what\r\n")?;
        }
    }
}

fn spawn_peer(peer: Peer) -> (u16, JoinHandle<Transcript>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (mut tcp, _) = listener.accept().unwrap();
        tcp.set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        let mut received = String::new();
        match peer {
            Peer::Implicit(leaf) => {
                tls_session(tcp, leaf, peer, true, &mut received);
            }
            Peer::RejectsAuth | Peer::RejectsRecipient => {
                tls_session(tcp, "localhost", peer, true, &mut received);
            }
            Peer::Starttls(leaf) => {
                say(&mut tcp, "220 localhost ESMTP\r\n");
                session(&mut tcp, peer, true, &mut received);
                if received.contains("STARTTLS") {
                    tls_session(tcp, leaf, peer, false, &mut received);
                }
            }
            Peer::Stripped => {
                say(&mut tcp, "220 localhost ESMTP\r\n");
                session(&mut tcp, peer, false, &mut received);
            }
            Peer::BrokenHandshake => {
                say(&mut tcp, "220 localhost ESMTP\r\n");
                session(&mut tcp, peer, true, &mut received);
                let _ = tcp.write_all(b"this is not a TLS server hello\r\n");
                let mut rest = Vec::new();
                let _ = tcp.read_to_end(&mut rest);
                received.push_str(&String::from_utf8_lossy(&rest));
            }
        }
        Transcript { received }
    });
    (port, handle)
}

fn tls_session(tcp: TcpStream, leaf: &str, peer: Peer, greet: bool, received: &mut String) {
    let connection = ServerConnection::new(server_config(leaf)).unwrap();
    let mut stream = StreamOwned::new(connection, tcp);
    if greet && say(&mut stream, "220 localhost ESMTP\r\n").is_none() {
        return;
    }
    // After STARTTLS the client sends EHLO again; no greeting.
    session(&mut stream, peer, false, received);
}

fn smtp_config(port: u16, tls: &str) -> SmtpConfig {
    let port = port.to_string();
    let vars = [
        ("KOHAKU_SMTP_HOST", "localhost"),
        ("KOHAKU_SMTP_PORT", port.as_str()),
        ("KOHAKU_SMTP_TLS", tls),
        ("KOHAKU_SMTP_USERNAME", USERNAME),
        ("KOHAKU_SMTP_PASSWORD", PASSWORD),
    ];
    load_smtp(|name| {
        vars.iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| (*v).into())
    })
    .unwrap()
}

fn ehlo() -> DnsName {
    DnsName::parse("kohaku.example.org").unwrap()
}

fn outgoing() -> Outgoing {
    let sender = serve_config(&[]).sender;
    let body = "Grüße from Kohaku.";
    Outgoing {
        row_id: 1,
        message: build_message(&sender, "reporter@example.com", "Hello", body).unwrap(),
        body: body.to_owned(),
    }
}

fn mailer(port: u16, tls: &str, roots: SmtpRoots) -> SmtpMailer {
    SmtpMailer::new(&smtp_config(port, tls), &ehlo(), roots).unwrap()
}

fn test_roots() -> SmtpRoots {
    SmtpRoots::for_tests(&fixture("ca.pem"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verified_server_gets_the_message() {
    for (peer, tls) in [
        (Peer::Implicit("localhost"), "implicit"),
        (Peer::Starttls("localhost"), "starttls"),
    ] {
        let (port, handle) = spawn_peer(peer);
        mailer(port, tls, test_roots())
            .send(outgoing())
            .await
            .unwrap();
        let transcript = tokio::task::spawn_blocking(move || handle.join().unwrap())
            .await
            .unwrap();
        let text = transcript.received;
        assert!(
            text.contains("MAIL FROM:<kohaku@kohaku.example.org>"),
            "{text}"
        );
        assert_eq!(text.matches("RCPT TO:").count(), 1, "{text}");
        assert!(text.contains("RCPT TO:<reporter@example.com>"), "{text}");
        assert!(
            text.contains("Content-Type: text/plain; charset=utf-8"),
            "{text}"
        );
        assert!(text.contains("From: kohaku@kohaku.example.org"), "{text}");
        assert_eq!(text.matches("\r\nTo: ").count(), 1, "{text}");
        assert!(!text.to_ascii_lowercase().contains("multipart"), "{text}");
        if tls == "starttls" {
            let before_tls = text.split("STARTTLS").next().unwrap();
            assert!(!before_tls.contains("AUTH"), "{text}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unverifiable_certificates_fail_before_auth() {
    let cases = [
        (Peer::Implicit("self-signed"), "implicit", test_roots()),
        (Peer::Implicit("expired"), "implicit", test_roots()),
        (Peer::Implicit("other-host"), "implicit", test_roots()),
        (
            Peer::Implicit("localhost"),
            "implicit",
            SmtpRoots::compiled(),
        ),
        (Peer::Starttls("self-signed"), "starttls", test_roots()),
        (Peer::Starttls("expired"), "starttls", test_roots()),
        (Peer::Starttls("other-host"), "starttls", test_roots()),
        (
            Peer::Starttls("localhost"),
            "starttls",
            SmtpRoots::compiled(),
        ),
    ];
    for (peer, tls, roots) in cases {
        let (port, handle) = spawn_peer(peer);
        let failure = mailer(port, tls, roots).send(outgoing()).await.unwrap_err();
        // lettre reports a failed handshake as a connection error.
        assert!(
            matches!(failure.class, FailureClass::Tls | FailureClass::Connection),
            "{peer:?}: {failure:?}"
        );
        let transcript = tokio::task::spawn_blocking(move || handle.join().unwrap())
            .await
            .unwrap();
        transcript.saw_nothing_after_tls();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attacker_strips_starttls_or_breaks_the_handshake() {
    for peer in [Peer::Stripped, Peer::BrokenHandshake] {
        let (port, handle) = spawn_peer(peer);
        let failure = mailer(port, "starttls", test_roots())
            .send(outgoing())
            .await
            .unwrap_err();
        assert!(
            matches!(
                failure.class,
                FailureClass::Tls | FailureClass::Protocol | FailureClass::Connection
            ),
            "{peer:?}: {failure:?}"
        );
        let transcript = tokio::task::spawn_blocking(move || handle.join().unwrap())
            .await
            .unwrap();
        transcript.saw_nothing_after_tls();
    }
}

#[tokio::test]
async fn construction_needs_no_server() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mailer = mailer(port, "implicit", SmtpRoots::compiled());
    let failure = mailer.send(outgoing()).await.unwrap_err();
    assert_eq!(failure.class, FailureClass::Connection);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejected_credentials_and_echoed_addresses() {
    for (peer, class, code) in [
        (Peer::RejectsAuth, FailureClass::Authentication, 535),
        (Peer::RejectsRecipient, FailureClass::Rejected, 550),
    ] {
        let (port, handle) = spawn_peer(peer);
        let failure = mailer(port, "implicit", test_roots())
            .send(outgoing())
            .await
            .unwrap_err();
        assert_eq!((failure.class, failure.code), (class, Some(code)));
        let debug = format!("{failure:?}");
        for secret in [
            PASSWORD,
            USERNAME,
            "victim@example.com",
            "Recipient address rejected",
            "credentials invalid",
        ] {
            assert!(!debug.contains(secret), "{debug}");
        }
        drop(tokio::task::spawn_blocking(move || handle.join()).await);
    }
}

#[test]
fn smtp_config_debug_holds_no_password() {
    let config = smtp_config(587, "starttls");
    let password = config.password.expose().to_owned();
    assert_eq!(password, PASSWORD);
    // SmtpConfig and SmtpPassword implement no Debug; the host and port are all a
    // caller can print.
    let shown = format!("{}:{}", config.host, config.port);
    assert!(!shown.contains(PASSWORD));
}

// The worker on the real adapter.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use kohaku::mail::outbox::{MailKind, NewMail, Priority, Recipient, Worker, enqueue};
use kohaku::routing::AppState;
use kohaku::routing::table::{HostKind, table};
use kohaku::time::now_unix;

static OTP: MailKind = MailKind {
    name: "test_otp",
    priority: Priority::Normal,
    token: false,
    lifetime: 3600,
    give_up: None,
};

static WORKER_KINDS: &[&MailKind] = &[&OTP];

/// A public route queuing one mail per request, like the OTP send of a later change.
async fn queue_mail(State(app): State<AppState>) -> StatusCode {
    let wakeup = app.outbox.clone();
    let queued = app
        .db
        .write(move |tx| {
            enqueue(
                tx,
                &wakeup,
                NewMail {
                    kind: &OTP,
                    recipient: Recipient::Address("victim@example.com".to_owned()),
                    subject: "Your code",
                    body: "Code 123456",
                    placeholder: false,
                },
                now_unix(),
            )
            .map_err(|_| rusqlite::Error::InvalidQuery)
        })
        .await;
    match queued {
        Ok(_) => StatusCode::ACCEPTED,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Accepts TCP connections and never answers; counts how many are open at once.
fn silent_listener() -> (u16, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let open = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let (open2, peak2) = (Arc::clone(&open), Arc::clone(&peak));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let now = open2.fetch_add(1, Ordering::SeqCst) + 1;
            peak2.fetch_max(now, Ordering::SeqCst);
            let open3 = Arc::clone(&open2);
            std::thread::spawn(move || {
                let mut sink = [0u8; 1024];
                while matches!(stream.read(&mut sink), Ok(n) if n > 0) {}
                open3.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    (port, open, peak)
}

fn start_worker(
    harness: &Harness,
    port: u16,
    attempt_timeout: Duration,
) -> tokio::sync::watch::Sender<()> {
    let (stop, stopped) = tokio::sync::watch::channel(());
    let worker = Worker {
        db: Arc::clone(&harness.app.db),
        mailer: Arc::new(mailer(port, "implicit", test_roots())),
        kinds: WORKER_KINDS,
        wakeup: harness.app.outbox.clone(),
        sender: serve_config(&[]).sender,
        attempt_timeout,
    };
    tokio::spawn(worker.run(stopped));
    stop
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stalled_smtp_server_does_not_hold_requests() {
    let dead = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let (silent, _open, peak) = silent_listener();
    for port in [dead, silent] {
        let mut routes = table();
        routes.push(synthetic_route(HostKind::Main, "/queue", post(queue_mail)));
        let harness = Harness::with(routes, &[]);
        let _stop = start_worker(&harness, port, Duration::from_secs(1));
        for _ in 0..30 {
            let started = Instant::now();
            let response = harness
                .send(
                    request("POST", MAIN_HOST, "/queue")
                        .header("origin", "https://kohaku.example.org")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await;
            assert_eq!(response.status(), StatusCode::ACCEPTED);
            assert!(started.elapsed() < Duration::from_millis(500));
        }
        tokio::time::sleep(Duration::from_millis(2500)).await;
        let unsent: i64 = harness
            .app
            .db
            .read(|c| {
                c.query_row(
                    "SELECT count(*) FROM outbox WHERE outcome IS NULL",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(unsent, 30);
    }
    assert_eq!(
        peak.load(Ordering::SeqCst),
        1,
        "never more than one SMTP connection"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn failed_attempts_are_logged_without_credentials_or_addresses() {
    let capture = LogCapture::default();
    let _guard = capture.install();
    let mut routes = table();
    routes.push(synthetic_route(HostKind::Main, "/queue", post(queue_mail)));
    let harness = Harness::with(routes, &[]);
    for peer in [Peer::RejectsAuth, Peer::RejectsRecipient] {
        let (port, handle) = spawn_peer(peer);
        let stop = start_worker(&harness, port, Duration::from_secs(10));
        harness
            .send(
                request("POST", MAIN_HOST, "/queue")
                    .header("origin", "https://kohaku.example.org")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        for _ in 0..200 {
            if capture.text().matches("outcome=\"failed\"").count()
                > usize::from(peer == Peer::RejectsRecipient)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let state: Vec<(i64, Option<String>)> = harness
            .app
            .db
            .read(|c| {
                c.prepare("SELECT attempts, outcome FROM outbox")?
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect()
            })
            .await
            .unwrap();
        assert!(
            state
                .iter()
                .all(|&(attempts, ref outcome)| attempts == 1 && outcome.is_none()),
            "{state:?}"
        );
        stop.send(()).unwrap();
        drop(tokio::task::spawn_blocking(move || handle.join()).await);
    }
    let log = capture.text();
    assert!(
        log.contains("class=\"authentication\"") && log.contains("code=535"),
        "{log}"
    );
    assert!(
        log.contains("class=\"rejected\"") && log.contains("code=550"),
        "{log}"
    );
    for secret in [
        PASSWORD,
        USERNAME,
        "victim@example.com",
        "Your code",
        "123456",
        "credentials invalid",
        "Recipient address rejected",
    ] {
        assert!(!log.contains(secret), "{secret} in {log}");
    }
}
