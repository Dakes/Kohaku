//! The command layer with its environment, data directory and streams injected
//! (operations: Backup and restore through standard streams, Listing backups,
//! Healthcheck command). Tests running the binary are in `binary.rs`.

mod support;

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use kohaku::cli::{BackupTarget, Command, CommandError, RestoreSource, execute};
use kohaku::db::backup::{BackupError, RestoreError};
use kohaku::db::lock::{InstanceLock, LockError};
use kohaku::db::migrate::{MIGRATIONS, open_and_prepare};
use kohaku::healthcheck::{Unhealthy, probe};
use support::*;

#[test]
fn every_problem_is_reported_in_one_run_and_the_data_directory_stays_empty() {
    let (dir, data) = data_dir();
    let mut vars = valid_environment();
    vars.remove("KOHAKU_TRUSTED_PROXIES");
    vars.insert("KOHAKU_SMTP_FROM".to_owned(), OsString::new());
    let error = execute(
        Command::Serve,
        &|n: &str| vars.get(n).cloned(),
        &data,
        &mut std::io::empty(),
        &mut Vec::new(),
    )
    .unwrap_err();
    let text = error.to_string();
    assert!(
        text.contains("KOHAKU_TRUSTED_PROXIES") && text.contains("KOHAKU_SMTP_FROM"),
        "{text}"
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

fn secret_env() -> HashMap<String, OsString> {
    HashMap::from([("KOHAKU_SECRET".to_owned(), OsString::from(TEST_SECRET))])
}

fn exec(
    command: Command,
    data: &kohaku::db::DataDir,
    stdin: &[u8],
) -> (Result<(), CommandError>, Vec<u8>) {
    let env = secret_env();
    let mut out = Vec::new();
    let result = execute(
        command,
        &|n: &str| env.get(n).cloned(),
        data,
        &mut &stdin[..],
        &mut out,
    );
    (result, out)
}

#[test]
fn round_trip_through_standard_streams() {
    let (_dir, data) = data_dir();
    let conn = open_and_prepare(&data, &secret(), MIGRATIONS, 1).unwrap();
    conn.execute_batch("CREATE TABLE marks (v TEXT) STRICT; INSERT INTO marks VALUES ('kept');")
        .unwrap();
    drop(conn);
    let (result, backup) = exec(Command::Backup(BackupTarget::Stdout), &data, b"");
    result.unwrap();
    assert!(backup.starts_with(b"SQLite format 3\0"));
    let files = TempDir::new();
    let path = files.path().join("b.db");
    let (result, out) = exec(
        Command::Backup(BackupTarget::File(path.clone())),
        &data,
        b"",
    );
    result.unwrap();
    assert!(out.is_empty());
    let (result, out) = exec(Command::Restore(RestoreSource::Stdin), &data, &backup);
    result.unwrap();
    assert!(out.is_empty());
    let (result, out) = exec(Command::Restore(RestoreSource::File(path)), &data, b"");
    result.unwrap();
    assert!(out.is_empty());
    let conn = rusqlite::Connection::open(data.database()).unwrap();
    let kept: String = conn
        .query_row("SELECT v FROM marks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(kept, "kept");
}

#[test]
fn broken_or_incomplete_streams() {
    let (_dir, data) = data_dir();
    drop(open_and_prepare(&data, &secret(), MIGRATIONS, 1).unwrap());
    let (result, backup) = exec(Command::Backup(BackupTarget::Stdout), &data, b"");
    result.unwrap();
    let before = fs::read(data.database()).unwrap();
    for input in [&[][..], &backup[..backup.len() / 2]] {
        let (result, _) = exec(Command::Restore(RestoreSource::Stdin), &data, input);
        assert!(matches!(
            result,
            Err(CommandError::Restore(RestoreError::NotABackup))
        ));
        assert_eq!(fs::read(data.database()).unwrap(), before);
    }
    struct Closing(usize);
    impl Write for Closing {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.0 + buf.len() > 4096 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "closed",
                ));
            }
            self.0 += buf.len();
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let env = secret_env();
    let result = execute(
        Command::Backup(BackupTarget::Stdout),
        &|n: &str| env.get(n).cloned(),
        &data,
        &mut std::io::empty(),
        &mut Closing(0),
    );
    let error = result.unwrap_err();
    assert!(matches!(
        error,
        CommandError::Backup(BackupError::Output(_))
    ));
    assert!(error.to_string().contains("standard output"));
}

#[test]
fn restore_refuses_while_the_lock_is_held() {
    let (_dir, data) = data_dir();
    drop(open_and_prepare(&data, &secret(), MIGRATIONS, 1).unwrap());
    let (_, backup) = exec(Command::Backup(BackupTarget::Stdout), &data, b"");
    let _held = InstanceLock::acquire(&data).unwrap();
    let (result, out) = exec(Command::Restore(RestoreSource::Stdin), &data, &backup);
    let error = result.unwrap_err();
    assert!(matches!(
        error,
        CommandError::Restore(RestoreError::Lock(LockError::Held))
    ));
    assert!(
        error
            .to_string()
            .contains("another Kohaku process is using /data")
    );
    assert!(out.is_empty());
}

#[test]
fn listing_prints_names_only() {
    let (_dir, data) = data_dir();
    let (result, out) = exec(Command::RestoreList, &data, b"");
    result.unwrap();
    assert!(out.is_empty());
    assert!(!data.backups().exists());
    let backups = data.ensure_backups().unwrap();
    fs::write(backups.join("pre-migrate-v1-1790000000.db"), b"").unwrap();
    fs::write(backups.join("2026-09-23.db"), b"").unwrap();
    let (result, out) = exec(Command::RestoreList, &data, b"");
    result.unwrap();
    assert_eq!(out, b"2026-09-23.db\npre-migrate-v1-1790000000.db\n");
}

// Healthcheck

/// A one-shot HTTP peer answering `response` (or nothing).
fn peer(response: Option<&'static str>) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf);
        match response {
            Some(text) => {
                let _ = stream.write_all(text.as_bytes());
            }
            None => std::thread::sleep(Duration::from_secs(8)),
        }
    });
    addr
}

#[test]
fn unhealthy_servers() {
    // Port 1 is privileged: nothing listens there and no test can bind it.
    let closed: std::net::SocketAddr = "127.0.0.1:1".parse().unwrap();
    assert_eq!(probe(closed), Err(Unhealthy::Refused));
    assert!(
        Unhealthy::Refused
            .to_string()
            .contains("connection refused")
    );
    let started = Instant::now();
    assert_eq!(probe(peer(None)), Err(Unhealthy::TimedOut));
    let elapsed = started.elapsed();
    // Below the peer's 8 s of silence, so the deadline ended it; slack for busy CI runners.
    assert!(
        elapsed >= Duration::from_millis(4900) && elapsed < Duration::from_secs(7),
        "{elapsed:?}"
    );
    assert_eq!(
        probe(peer(Some("HTTP/1.1 503 Service Unavailable\r\n\r\n"))),
        Err(Unhealthy::Status(503))
    );
    assert_eq!(
        probe(peer(Some(
            "HTTP/1.1 308 Permanent Redirect\r\nLocation: http://127.0.0.1:1/healthz\r\n\r\n"
        ))),
        Err(Unhealthy::Status(308))
    );
    assert_eq!(probe(peer(Some("garbage\r\n"))), Err(Unhealthy::Malformed));
    assert_eq!(probe(peer(Some("HTTP/1.1 200 OK\r\n\r\n"))), Ok(()));
    for reason in [
        Unhealthy::Refused,
        Unhealthy::TimedOut,
        Unhealthy::Status(503),
        Unhealthy::Malformed,
    ] {
        assert_eq!(reason.to_string().lines().count(), 1);
    }
}

fn project_env() -> HashMap<String, OsString> {
    let mut env = secret_env();
    env.insert(
        "KOHAKU_BASE_URL".to_owned(),
        OsString::from("https://kohaku.example.org"),
    );
    env
}

fn project_create(
    data: &kohaku::db::DataDir,
    slug: &str,
    name: &str,
    host: Option<&str>,
) -> (Result<(), String>, Vec<u8>) {
    let env = project_env();
    let mut out = Vec::new();
    let command = Command::ProjectCreate(kohaku::projects::commands::CreateArgs {
        slug: slug.into(),
        name: name.into(),
        host: host.map(OsString::from),
    });
    let result = execute(
        command,
        &|n: &str| env.get(n).cloned(),
        data,
        &mut std::io::empty(),
        &mut out,
    );
    (result.map_err(|e| e.to_string()), out)
}

fn project_rows(data: &kohaku::db::DataDir) -> Vec<(i64, String, String, Option<String>, bool)> {
    let conn = kohaku::db::open(&data.database(), kohaku::db::Role::Command).unwrap();
    conn.prepare("SELECT id, slug, name, public_host, plus_one_enabled FROM projects ORDER BY id")
        .unwrap()
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn audit_rows(data: &kohaku::db::DataDir) -> Vec<String> {
    let conn = kohaku::db::open(&data.database(), kohaku::db::Role::Command).unwrap();
    conn.prepare(
        "SELECT actor_label || ' ' || coalesce(actor_id, '-') || ' ' || action || ' '
             || target_type || ' ' || target_id FROM audit_log ORDER BY id",
    )
    .unwrap()
    .query_map([], |r| r.get(0))
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

#[test]
fn scripted_project_setup() {
    let (_dir, data) = data_dir();
    drop(open_and_prepare(&data, &secret(), MIGRATIONS, 1).unwrap());
    let (result, out) = project_create(&data, "demo", "Demo app", None);
    assert_eq!(result, Ok(()));
    assert!(out.is_empty(), "nothing on stdout");
    let (result, out) = project_create(&data, "demo", "Demo app", None);
    assert_eq!(result.unwrap_err(), "The slug is taken by another project.");
    assert!(out.is_empty());
    assert_eq!(
        project_rows(&data),
        [(1, "demo".to_owned(), "Demo app".to_owned(), None, true)]
    );
    assert_eq!(audit_rows(&data), ["cli - project.create project 1"]);

    let (result, _) = project_create(&data, "other", "Other", Some(" Bugs.Example.NET"));
    assert_eq!(result, Ok(()));
    assert_eq!(
        project_rows(&data)[1].3.as_deref(),
        Some("bugs.example.net")
    );
    // Each invalid value names its field; nothing is created or recorded.
    for (slug, name, host, field) in [
        ("Demo2", "Demo", None, "The slug"),
        ("-x", "Demo", None, "The slug"),
        ("x", "", None, "The name"),
        ("x", "a\u{202E}b", None, "The name"),
        ("x", "Demo", Some("bugs.example.net"), "The custom domain"),
        ("x", "Demo", Some("kohaku.example.org"), "The custom domain"),
        ("x", "Demo", Some("localhost"), "The custom domain"),
        ("x", "Demo", Some("192.0.2.1"), "The custom domain"),
        ("x", "Demo", Some(""), ""),
    ] {
        let (result, out) = project_create(&data, slug, name, host);
        if field.is_empty() {
            // An empty --host is no custom domain.
            assert_eq!(result, Ok(()));
            continue;
        }
        let message = result.unwrap_err();
        assert!(
            message.starts_with(field),
            "{slug} {name:?} {host:?}: {message}"
        );
        assert!(out.is_empty());
    }
    assert_eq!(project_rows(&data).len(), 3);
    assert_eq!(audit_rows(&data).len(), 3);
}

#[test]
fn non_utf8_project_values_name_their_field() {
    use std::os::unix::ffi::OsStringExt;
    let (_dir, data) = data_dir();
    drop(open_and_prepare(&data, &secret(), MIGRATIONS, 1).unwrap());
    let env = project_env();
    let bad = || OsString::from_vec(vec![b'a', 0xff]);
    for (args, field) in [
        ((bad(), "Demo".into(), None), "The slug"),
        (("demo".into(), bad(), None), "The name"),
        (
            ("demo".into(), "Demo".into(), Some(bad())),
            "The custom domain",
        ),
    ] {
        let (slug, name, host) = args;
        let command =
            Command::ProjectCreate(kohaku::projects::commands::CreateArgs { slug, name, host });
        let error = execute(
            command,
            &|n: &str| env.get(n).cloned(),
            &data,
            &mut std::io::empty(),
            &mut Vec::new(),
        )
        .unwrap_err();
        assert!(error.to_string().starts_with(field), "{error}");
    }
    assert!(project_rows(&data).is_empty());
}

#[test]
fn project_create_needs_its_settings_and_the_database() {
    let (_dir, data) = data_dir();
    let (result, _) = project_create(&data, "demo", "Demo", None);
    assert!(
        result.unwrap_err().contains("start `kohaku serve` first"),
        "no database is created"
    );
    assert!(!data.database().exists());
    let env = secret_env();
    let command = Command::ProjectCreate(kohaku::projects::commands::CreateArgs {
        slug: "demo".into(),
        name: "Demo".into(),
        host: None,
    });
    let error = execute(
        command,
        &|n: &str| env.get(n).cloned(),
        &data,
        &mut std::io::empty(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("KOHAKU_BASE_URL"), "{error}");
}
