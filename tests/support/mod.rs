//! Helpers shared by the integration tests.

#![allow(dead_code, reason = "each test binary uses a different subset")]

#[path = "../../src/test_support.rs"]
mod shared;

pub use shared::*;

use kohaku::db::DataDir;
use kohaku::keys::InstanceSecret;

pub fn secret() -> InstanceSecret {
    InstanceSecret::from_base64(TEST_SECRET).unwrap()
}

pub fn other_secret() -> InstanceSecret {
    InstanceSecret::from_base64(OTHER_TEST_SECRET).unwrap()
}

/// A temporary data directory; keep the `TempDir` alive as long as the `DataDir`.
pub fn data_dir() -> (TempDir, DataDir) {
    let dir = TempDir::new();
    let data = DataDir::new(dir.path());
    (dir, data)
}

/// Mode bits of `path`.
pub fn mode(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o7777
}

/// The environment variable that turns an `#[ignore]`d child test into a helper
/// process; it holds the child's argument.
pub const CHILD_ENV: &str = "KOHAKU_TEST_CHILD";

/// The argument of a child helper, `None` when the test runs normally.
#[expect(
    clippy::disallowed_methods,
    reason = "test helper processes get their argument from the environment"
)]
pub fn child_argument() -> Option<String> {
    std::env::var(CHILD_ENV).ok()
}

/// What a child helper prints once ready; libtest may put it after its own text.
const READY: &str = "kohaku-child-ready";

/// Re-runs this test binary as a helper process executing only the ignored test
/// `name` with `argument`; returns once the child is ready.
pub fn spawn_child(name: &str, argument: &str) -> std::process::Child {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            name,
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, argument)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut lines = BufReader::new(stdout).lines();
    loop {
        match lines.next() {
            Some(Ok(line)) if line.ends_with(READY) => break,
            Some(Ok(_)) => continue,
            other => panic!("child {name} ended before it was ready: {other:?}"),
        }
    }
    // Keep the pipe open so the child never gets SIGPIPE.
    std::mem::forget(lines);
    child
}

/// Tells the parent a child helper is ready, then waits to be killed.
pub fn child_ready_and_wait() -> ! {
    use std::io::Write;
    println!("{READY}");
    std::io::stdout().flush().unwrap();
    loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
}

// HTTP harness

use std::collections::HashMap;
use std::ffi::OsString;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, Response};
use kohaku::cli::Command;
use kohaku::config::{Config, ServeConfig};
use kohaku::db::Db;
use kohaku::db::migrate::{MIGRATIONS, open_and_prepare};
use kohaku::http::PeerAddr;
use kohaku::routing::build::{Service_, build};
use kohaku::routing::table::{Route, table};
use kohaku::routing::{App, AppState};
use tower::ServiceExt;

/// The main host of [`valid_environment`].
pub const MAIN_HOST: &str = "kohaku.example.org";

/// The peer of requests that name none.
pub const DEFAULT_PEER: &str = "203.0.113.5:40000";

/// A `serve` configuration: the valid environment with `changes` applied.
pub fn serve_config(changes: &[(&str, Option<&str>)]) -> ServeConfig {
    let mut vars: HashMap<String, OsString> = valid_environment();
    for (name, value) in changes {
        match value {
            Some(value) => vars.insert((*name).to_owned(), OsString::from(value)),
            None => vars.remove(*name),
        };
    }
    match Config::load(&Command::Serve, |name| vars.get(name).cloned()) {
        Ok(Config::Serve(config)) => config,
        Ok(_) => unreachable!("serve loads a serve configuration"),
        Err(error) => panic!("invalid test configuration: {error}"),
    }
}

/// An app over a migrated database in a temporary directory, and its service.
pub struct Harness {
    pub app: AppState,
    pub service: Service_,
    pub data: DataDir,
    _dir: TempDir,
}

impl Harness {
    pub fn new() -> Harness {
        Harness::with(table(), &[])
    }

    pub fn with(routes: Vec<Route>, changes: &[(&str, Option<&str>)]) -> Harness {
        let config = serve_config(changes);
        let (dir, data) = data_dir();
        let writer = open_and_prepare(&data, &config.secret, MIGRATIONS, 1_800_000_000).unwrap();
        let db = Arc::new(Db::new(writer, &data.database()).unwrap());
        let app = Arc::new(App::new(&config, db, kohaku::time::Clock::manual()).unwrap());
        let service = build(routes, &app);
        Harness {
            app,
            service,
            data,
            _dir: dir,
        }
    }

    /// Sends `request`, from [`DEFAULT_PEER`] unless it carries a `PeerAddr`.
    pub async fn send(&self, mut request: Request<Body>) -> Response<Body> {
        if request.extensions().get::<PeerAddr>().is_none() {
            request
                .extensions_mut()
                .insert(PeerAddr(DEFAULT_PEER.parse().unwrap()));
        }
        self.service.clone().oneshot(request).await.unwrap()
    }

    pub async fn get(&self, host: &str, path: &str) -> Response<Body> {
        self.send(request("GET", host, path).body(Body::empty()).unwrap())
            .await
    }
}

/// A request builder for `method` and `path` with `Host: host` (none when empty).
pub fn request(method: &str, host: &str, path: &str) -> axum::http::request::Builder {
    let builder = Request::builder().method(method).uri(path);
    if host.is_empty() {
        builder
    } else {
        builder.header("host", host)
    }
}

/// A request from `peer`.
pub fn from_peer(mut request: Request<Body>, peer: &str) -> Request<Body> {
    let peer: SocketAddr = peer.parse().unwrap();
    request.extensions_mut().insert(PeerAddr(peer));
    request
}

pub async fn body_text(response: Response<Body>) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// Every value of a response header, as text.
pub fn header_values(response: &Response<Body>, name: &str) -> Vec<String> {
    response
        .headers()
        .get_all(name)
        .iter()
        .map(|v| v.to_str().unwrap().to_owned())
        .collect()
}

/// A synthetic public route with the default declarations, for pipeline tests.
pub fn synthetic_route(
    host: kohaku::routing::table::HostKind,
    pattern: &'static str,
    methods: axum::routing::MethodRouter<AppState>,
) -> Route {
    use kohaku::routing::table::*;
    Route {
        host,
        path: RoutePath::Pattern(pattern),
        example: pattern.to_owned(),
        methods,
        access: Access::Public,
        cache: CacheClass::NoStore,
        csp: Csp::Release,
        body: BodyClass::Default,
        multipart: Multipart::Rejected,
        headerless: Headerless::NotExempt,
        rate: &[],
        errors: ErrorFormat::Html,
    }
}

/// A real server on 127.0.0.1 over a harness, stopped when dropped.
pub struct Server {
    pub addr: SocketAddr,
    pub harness: Harness,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    pub task: Option<tokio::task::JoinHandle<()>>,
}

impl Server {
    pub async fn start(harness: Harness) -> Server {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(kohaku::http::server::run(
            listener,
            harness.service.clone(),
            Arc::clone(&harness.app),
            async move {
                let _ = stopped.await;
            },
        ));
        Server {
            addr,
            harness,
            stop: Some(stop),
            task: Some(task),
        }
    }

    /// A blocking raw-socket connection to the server.
    pub fn connect(&self) -> std::net::TcpStream {
        let stream = std::net::TcpStream::connect(self.addr).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(30)))
            .unwrap();
        stream
    }

    /// Starts graceful shutdown; await `task` to see it finish.
    pub fn shutdown(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Sends raw bytes on a new connection and reads until the server closes it or 30 s
/// pass.
pub fn raw_exchange(addr: SocketAddr, bytes: &[u8]) -> String {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(30)))
        .unwrap();
    stream.write_all(bytes).unwrap();
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);
    String::from_utf8_lossy(&response).into_owned()
}

/// Log output captured on the current thread. One global subscriber at TRACE (the
/// most verbose level) writes into the buffer of the thread that logs, so parallel
/// tests neither share output nor disturb tracing's per-callsite interest cache.
#[derive(Clone, Default)]
pub struct LogCapture(Arc<std::sync::Mutex<Vec<u8>>>);

thread_local! {
    static CAPTURE: std::cell::RefCell<Option<Arc<std::sync::Mutex<Vec<u8>>>>> =
        const { std::cell::RefCell::new(None) };
}

pub struct LogWriter;

impl std::io::Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        CAPTURE.with(|capture| {
            if let Some(buffer) = capture.borrow().as_ref() {
                buffer.lock().unwrap().extend_from_slice(buf);
            }
        });
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Stops capturing on this thread when dropped.
pub struct CaptureGuard;

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        CAPTURE.with(|capture| *capture.borrow_mut() = None);
    }
}

impl LogCapture {
    /// Captures everything logged on this thread until the guard drops.
    pub fn install(&self) -> CaptureGuard {
        static GLOBAL: std::sync::Once = std::sync::Once::new();
        GLOBAL.call_once(|| {
            let subscriber = tracing_subscriber::fmt()
                .with_ansi(false)
                .with_max_level(tracing::Level::TRACE)
                .with_writer(|| LogWriter)
                .finish();
            tracing::subscriber::set_global_default(subscriber).unwrap();
        });
        CAPTURE.with(|capture| *capture.borrow_mut() = Some(Arc::clone(&self.0)));
        CaptureGuard
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

/// One request over a new connection with `Connection: close`; the status code, or
/// `None` when the server closed without answering.
pub async fn wire_request(
    addr: SocketAddr,
    method: &str,
    host: &str,
    path: &str,
    headers: &[(&str, &str)],
) -> Option<u16> {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    write_all(&stream, head.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    let mut buf = [0u8; 4096];
    while let Ok(n) = read_some(&stream, &mut buf).await {
        if n == 0 {
            break;
        }
        response.extend_from_slice(&buf[..n]);
    }
    let text = String::from_utf8_lossy(&response);
    text.strip_prefix("HTTP/1.1 ")?.get(..3)?.parse().ok()
}

/// Writes all of `bytes` (tokio's `io-util` is not a dependency).
pub async fn write_all(stream: &tokio::net::TcpStream, mut bytes: &[u8]) -> std::io::Result<()> {
    while !bytes.is_empty() {
        stream.writable().await?;
        match stream.try_write(bytes) {
            Ok(n) => bytes = &bytes[n..],
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Reads what is available, waiting for at least one byte; 0 at end of stream.
pub async fn read_some(stream: &tokio::net::TcpStream, buf: &mut [u8]) -> std::io::Result<usize> {
    loop {
        stream.readable().await?;
        match stream.try_read(buf) {
            Ok(n) => return Ok(n),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) => return Err(e),
        }
    }
}

// Accounts and sessions (change admin-auth D13)

use kohaku::auth::totp::{Seed, code, step_at};

/// The password of every test account.
pub const PASSWORD: &str = "correct horse battery staple";

/// An account written directly into the database.
pub struct TestAccount {
    pub id: i64,
    pub email: String,
    pub nonce: Option<[u8; 16]>,
}

/// A browser: its cookies, the session's CSRF token and its own network address, so
/// tests stay within the per-network login budget.
#[derive(Clone)]
pub struct Browser {
    pub session: Option<String>,
    pub device: Option<String>,
    pub csrf: String,
    pub peer: String,
}

impl Browser {
    /// A browser that never signed in, at a new address.
    pub fn new() -> Browser {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Browser {
            session: None,
            device: None,
            csrf: String::new(),
            // 198.18.0.0/15, the benchmarking range: 131 072 addresses.
            peer: format!(
                "198.{}.{}.{}:40000",
                18 + (n >> 16) % 2,
                (n >> 8) & 255,
                n & 255
            ),
        }
    }

    pub fn cookie_header(&self) -> String {
        let mut cookies = Vec::new();
        if let Some(session) = &self.session {
            cookies.push(format!("__Host-kohaku_session={session}"));
        }
        if let Some(device) = &self.device {
            cookies.push(format!("__Host-kohaku_device={device}"));
        }
        cookies.join("; ")
    }

    /// Takes the cookies a response sets.
    pub fn absorb(&mut self, response: &Response<Body>) {
        for value in header_values(response, "set-cookie") {
            let pair = value.split(';').next().unwrap();
            let (name, token) = pair.split_once('=').unwrap();
            let token = (!token.is_empty()).then(|| token.to_owned());
            match name {
                "__Host-kohaku_session" => self.session = token,
                "__Host-kohaku_device" => self.device = token,
                other => panic!("unexpected cookie {other}"),
            }
        }
    }
}

impl Default for Browser {
    fn default() -> Browser {
        Browser::new()
    }
}

/// Percent-encodes a form value.
pub fn form_encode(fields: &[(&str, &str)]) -> String {
    let encode = |text: &str| {
        text.bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                    char::from(b).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect::<String>()
    };
    fields
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// A same-origin form POST from `browser` to the main host.
pub fn form_request(path: &str, browser: &Browser, fields: &[(&str, &str)]) -> Request<Body> {
    let mut builder = request("POST", MAIN_HOST, path)
        .header("origin", format!("https://{MAIN_HOST}"))
        .header("sec-fetch-site", "same-origin")
        .header("content-type", "application/x-www-form-urlencoded");
    let cookies = browser.cookie_header();
    if !cookies.is_empty() {
        builder = builder.header("cookie", cookies);
    }
    from_peer(
        builder.body(Body::from(form_encode(fields))).unwrap(),
        &browser.peer,
    )
}

/// The value of the first `name="csrf"` field of a page.
pub fn csrf_of(page: &str) -> String {
    let at = page
        .find("name=\"csrf\" value=\"")
        .expect("the page has a CSRF field");
    let rest = &page[at + "name=\"csrf\" value=\"".len()..];
    rest[..rest.find('"').unwrap()].to_owned()
}

impl Harness {
    /// Wall time of the harness clock.
    pub fn now(&self) -> i64 {
        self.app.clock.unix()
    }

    pub fn advance(&self, seconds: u64) {
        self.app
            .clock
            .advance(std::time::Duration::from_secs(seconds));
    }

    /// Runs `sql` with `params` on the writer.
    pub async fn exec(&self, sql: &'static str, params: Vec<rusqlite::types::Value>) -> usize {
        self.app
            .db
            .write(move |tx| tx.execute(sql, rusqlite::params_from_iter(params)))
            .await
            .unwrap()
    }

    /// One integer from `sql` on a reader.
    pub async fn count(&self, sql: &'static str) -> i64 {
        self.app
            .db
            .read(move |c| c.query_row(sql, [], |r| r.get::<_, i64>(0)))
            .await
            .unwrap()
    }

    /// A new account with [`PASSWORD`], TOTP enrolled when `totp`.
    pub async fn account(&self, email: &str, role: &str, totp: bool) -> TestAccount {
        let hash = kohaku::auth::password::hash_password(PASSWORD).unwrap();
        let nonce = totp.then(|| {
            let mut nonce = [0u8; 16];
            kohaku::keys::random_bytes(&mut nonce).unwrap();
            nonce
        });
        let (email_owned, role, now) = (email.to_owned(), role.to_owned(), self.now());
        let id = self
            .app
            .db
            .write(move |tx| {
                tx.query_row(
                    "INSERT INTO users (email, password_hash, role, totp_nonce, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5) RETURNING id",
                    rusqlite::params![email_owned, hash, role, nonce.map(|n| n.to_vec()), now],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        TestAccount {
            id,
            email: email.to_owned(),
            nonce,
        }
    }

    /// The account's TOTP code `offset` steps from now.
    pub fn code(&self, account: &TestAccount, offset: i64) -> String {
        let seed = Seed::derive(
            &secret(),
            account.id,
            &account.nonce.expect("TOTP enrolled"),
        );
        code(&seed, step_at(self.now()) + offset)
    }

    /// Posts the login form from `browser` and takes its cookies.
    pub async fn login_as(
        &self,
        browser: &mut Browser,
        email: &str,
        password: &str,
        code: &str,
    ) -> Response<Body> {
        let response = self
            .send(form_request(
                "/admin/login",
                browser,
                &[("email", email), ("password", password), ("code", code)],
            ))
            .await;
        browser.absorb(&response);
        response
    }

    /// Signs `account` in from a new browser with a fresh code, and reads its CSRF
    /// token; the clock moves one step on, so the next code is fresh too.
    pub async fn sign_in(&self, account: &TestAccount) -> Browser {
        let mut browser = Browser::new();
        let code = account
            .nonce
            .map(|_| self.code(account, 0))
            .unwrap_or_default();
        let response = self
            .login_as(&mut browser, &account.email, PASSWORD, &code)
            .await;
        assert_eq!(response.status(), 303, "sign-in of {}", account.email);
        self.advance(30);
        let page = self.get_as(&browser, "/admin").await;
        browser.csrf = csrf_of(&body_text(page).await);
        browser
    }

    /// A same-origin GET from `browser`.
    pub async fn get_as(&self, browser: &Browser, path: &str) -> Response<Body> {
        let mut builder = request("GET", MAIN_HOST, path);
        let cookies = browser.cookie_header();
        if !cookies.is_empty() {
            builder = builder.header("cookie", cookies);
        }
        self.send(from_peer(
            builder.body(Body::empty()).unwrap(),
            &browser.peer,
        ))
        .await
    }

    /// A form POST from `browser` with its CSRF token added.
    pub async fn post_as(
        &self,
        browser: &Browser,
        path: &str,
        fields: &[(&str, &str)],
    ) -> Response<Body> {
        let mut all = vec![("csrf", browser.csrf.as_str())];
        all.extend_from_slice(fields);
        self.send(form_request(path, browser, &all)).await
    }
}
