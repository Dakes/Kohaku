//! The `kohaku` binary as a process (operations: Command-line commands, Usage errors,
//! Healthcheck command, Secrets are never arguments or output). Its own test binary:
//! forking children must not race the lock tests.

mod support;

use std::fs;
use std::process::{Command as Process, Output, Stdio};

use kohaku::cli::USAGE;
use support::*;

/// Runs the binary in `cwd` with only `env` set.
fn run(args: &[&str], env: &[(&str, &str)], cwd: &std::path::Path) -> Output {
    Process::new(env!("CARGO_BIN_EXE_kohaku"))
        .args(args)
        .env_clear()
        .envs(env.iter().copied())
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn entries(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn valid_env() -> Vec<(String, String)> {
    valid_environment()
        .into_iter()
        .map(|(k, v)| (k, v.into_string().unwrap()))
        .collect()
}

#[test]
fn version_and_help_need_no_configuration() {
    let dir = TempDir::new();
    let version = run(&["--version"], &[], dir.path());
    assert!(version.status.success());
    assert_eq!(
        version.stdout,
        format!("kohaku {}\n", env!("CARGO_PKG_VERSION")).as_bytes()
    );
    assert!(version.stderr.is_empty());
    for args in [
        &["--help"][..],
        &["backup", "--help"],
        &["serve", "--help"],
        &["healthcheck", "--help"],
        &["restore", "--help"],
    ] {
        let help = run(args, &[], dir.path());
        assert!(help.status.success(), "{args:?}");
        let text = String::from_utf8(help.stdout).unwrap();
        assert_eq!(text, USAGE);
        for name in [
            "serve",
            "healthcheck",
            "backup",
            "restore",
            "restore --list",
        ] {
            assert!(text.contains(name));
        }
    }
    assert!(entries(dir.path()).is_empty(), "no file created");
}

#[test]
fn malformed_invocations() {
    let dir = TempDir::new();
    let env = valid_env();
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let malformed: &[&[&str]] = &[
        &[],
        &["frobnicate"],
        &["serve", "--port", "9000"],
        &["backup"],
        &["restore", "--list", "extra"],
        &["restore", "a.db", "b.db"],
        &["SERVE"],
        &["serve", "--secret", "abc"],
        &["serve", "--password", "abc"],
        &["healthcheck", "extra"],
    ];
    for args in malformed {
        for vars in [&env[..], &[]] {
            let output = run(args, vars, dir.path());
            assert_eq!(output.status.code(), Some(2), "{args:?}");
            assert!(output.stdout.is_empty());
            assert_eq!(String::from_utf8(output.stderr).unwrap(), USAGE);
        }
    }
    assert!(entries(dir.path()).is_empty());
}

#[test]
fn refusal_is_a_failure_not_a_usage_error() {
    let dir = TempDir::new();
    fs::write(dir.path().join("existing.db"), b"keep").unwrap();
    let output = run(
        &["backup", "existing.db"],
        &[("KOHAKU_SECRET", TEST_SECRET)],
        dir.path(),
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("existing.db already exists"), "{stderr}");
    assert!(!stderr.contains(TEST_SECRET));
    assert_eq!(fs::read(dir.path().join("existing.db")).unwrap(), b"keep");
}

#[test]
fn backup_without_its_secret_names_it_and_writes_nothing() {
    let dir = TempDir::new();
    let output = run(&["backup", "-"], &[], dir.path());
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("KOHAKU_SECRET"), "{stderr}");
    for secret in ["not-base64!", "c2hvcnQ="] {
        let output = run(&["backup", "-"], &[("KOHAKU_SECRET", secret)], dir.path());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            stderr.contains("KOHAKU_SECRET") && !stderr.contains(secret),
            "{stderr}"
        );
    }
    assert!(entries(dir.path()).is_empty());
}

#[test]
fn serve_logs_its_failure_on_stdout() {
    // No data directory here: the release build's /data and a dev build's ./data are
    // missing, so startup fails after validating the configuration.
    let dir = TempDir::new();
    let env = valid_env();
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let output = run(&["serve"], &env, dir.path());
    assert_eq!(output.status.code(), Some(1));
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("ERROR"), "{stdout}");
    for (_, value) in &env {
        if value.len() > 8 {
            assert!(!stdout.contains(value), "{value} in {stdout}");
        }
    }
}

#[test]
fn healthcheck_binary_reports_one_line_on_stderr() {
    assert!(
        std::net::TcpStream::connect(("127.0.0.1", 8080)).is_err(),
        "port 8080 is in use (a running `just dev`?); stop it to run this test"
    );
    let dir = TempDir::new();
    let proxies = [
        ("HTTP_PROXY", "http://192.0.2.1:9"),
        ("http_proxy", "http://192.0.2.1:9"),
        ("ALL_PROXY", "http://192.0.2.1:9"),
    ];
    let output = run(&["healthcheck"], &proxies, dir.path());
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(stderr.contains("connection refused"), "{stderr}");
    assert!(entries(dir.path()).is_empty());
}

/// The real `serve` binary answers a page and stops on SIGTERM, logging on stdout. A
/// `dev` build's data directory is `./data`, so it runs in a temporary directory.
#[cfg(feature = "dev")]
#[test]
fn serve_binary_answers_logs_on_stdout_and_stops_on_sigterm() {
    use std::io::{Read, Write};
    use std::time::{Duration, Instant};
    assert!(
        std::net::TcpStream::connect(("127.0.0.1", 8080)).is_err(),
        "port 8080 is in use (a running `just dev`?); stop it to run this test"
    );
    let dir = TempDir::new();
    fs::create_dir(dir.path().join("data")).unwrap();
    let child = Process::new(env!("CARGO_BIN_EXE_kohaku"))
        .arg("serve")
        .env_clear()
        .env("KOHAKU_BASE_URL", "http://localhost:8080")
        .env("KOHAKU_TRUSTED_PROXIES", "none")
        .env("KOHAKU_SECRET", TEST_SECRET)
        .env("KOHAKU_SMTP_FROM", "kohaku@localhost")
        .current_dir(dir.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    let mut stream = loop {
        match std::net::TcpStream::connect(("127.0.0.1", 8080)) {
            Ok(stream) => break stream,
            Err(_) if started.elapsed() < Duration::from_secs(20) => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(error) => panic!("serve did not start: {error}"),
        }
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost:8080\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let status = Process::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("listening on port 8080") && stdout.contains("route=\"/\""),
        "{stdout}"
    );
    assert!(!dir.path().join("data/kohaku.db-wal").exists());
}
