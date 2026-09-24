//! Password reset, the account commands and the account mails (admin-auth: Password
//! reset, Account commands, Account mail content; request-limits: Per-address reset
//! limit; change admin-auth D9–D11).

mod support;

use std::ffi::OsString;
use std::sync::Arc;

use axum::body::Body;
use axum::http::StatusCode;
use kohaku::cli::{Command, execute};
use kohaku::mail::outbox::{KINDS, give_up, pick};
use rusqlite::types::Value;
use support::*;

/// Asks for a reset from a new browser.
async fn ask(harness: &Harness, email: &str) -> (StatusCode, String) {
    let response = harness
        .send(form_request(
            "/admin/reset",
            &Browser::new(),
            &[("email", email)],
        ))
        .await;
    (response.status(), body_text(response).await)
}

/// The reset link in the newest token-bearing outbox row.
async fn mailed_link(harness: &Harness) -> String {
    let body: String = harness
        .app
        .db
        .read(|c| {
            c.query_row(
                "SELECT body FROM outbox WHERE kind = 'account_reset_link' AND placeholder = 0
                 ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
        })
        .await
        .unwrap();
    let start = body
        .find("https://kohaku.example.org/admin/reset/")
        .unwrap();
    body[start..].lines().next().unwrap().to_owned()
}

fn path_of(link: &str) -> &str {
    link.strip_prefix("https://kohaku.example.org").unwrap()
}

async fn set_password(
    harness: &Harness,
    path: &str,
    password: &str,
) -> (StatusCode, String, Vec<String>) {
    let response = harness
        .send(form_request(
            path,
            &Browser::new(),
            &[("password", password), ("password_again", password)],
        ))
        .await;
    let cookies = header_values(&response, "set-cookie");
    (response.status(), body_text(response).await, cookies)
}

async fn password_hash(harness: &Harness, id: i64) -> Option<String> {
    let db = Arc::clone(&harness.app.db);
    db.read(move |c| {
        c.query_row("SELECT password_hash FROM users WHERE id = ?1", [id], |r| {
            r.get(0)
        })
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn attacker_probes_accounts_through_reset() {
    let harness = Harness::new();
    harness.account("admin@example.org", "admin", true).await;
    let disabled = harness
        .account("off@example.org", "maintainer", false)
        .await;
    harness.account("on@example.org", "maintainer", false).await;
    harness
        .exec(
            "UPDATE users SET disabled = 1 WHERE id = ?1",
            vec![Value::Integer(disabled.id)],
        )
        .await;
    let mut answers = Vec::new();
    for email in [
        "nobody@example.org",
        "admin@example.org",
        "off@example.org",
        "on@example.org",
    ] {
        answers.push(ask(&harness, email).await);
    }
    assert_eq!(answers[0].0, StatusCode::OK);
    assert!(answers[0].1.contains("Check your mail"));
    for answer in &answers[1..] {
        assert_eq!(answer, &answers[0]);
    }
    assert_eq!(harness.count("SELECT count(*) FROM outbox").await, 4);
    assert_eq!(
        harness
            .count("SELECT count(*) FROM outbox WHERE token = 1 AND placeholder = 0")
            .await,
        1
    );
    assert_eq!(
        harness
            .count("SELECT count(*) FROM outbox WHERE placeholder = 1")
            .await,
        3
    );
    assert_eq!(harness.count("SELECT count(*) FROM tokens").await, 1);
    let stored = harness
        .count(
            "SELECT count(*) FROM outbox WHERE address LIKE '%example.org%'
                 OR body LIKE '%nobody%' OR body LIKE '%off@%' OR body LIKE '%admin@%'",
        )
        .await;
    assert_eq!(stored, 0, "no row names a requested address");
    // The worker deletes the placeholders without sending them.
    let now = harness.now();
    let picked = harness
        .app
        .db
        .write(move |tx| pick(tx, KINDS, now + 1))
        .await
        .unwrap();
    assert!(
        matches!(picked, kohaku::mail::outbox::Pick::Due(ref row) if row.address == "on@example.org")
    );
    assert_eq!(harness.count("SELECT count(*) FROM outbox").await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn attacker_with_the_victims_mailbox() {
    let harness = Harness::new();
    let account = harness.account("m@example.org", "maintainer", true).await;
    harness
        .exec(
            "INSERT INTO recovery_codes (user_id, code_hash) VALUES (?1, ?2)",
            vec![Value::Integer(account.id), Value::Blob(vec![1; 32])],
        )
        .await;
    let session = harness.sign_in(&account).await;
    harness
        .exec(
            "UPDATE users SET failed_logins = 3, locked_until = 5 WHERE id = ?1",
            vec![Value::Integer(account.id)],
        )
        .await;
    let state = "SELECT hex(totp_nonce) || ' ' || failed_logins || ' ' || locked_until
                     || ' ' || (SELECT count(*) FROM recovery_codes) FROM users";
    let db = Arc::clone(&harness.app.db);
    let before: String = db
        .read(move |c| c.query_row(state, [], |r| r.get(0)))
        .await
        .unwrap();
    ask(&harness, "M@Example.org ").await;
    let link = mailed_link(&harness).await;
    assert!(link.starts_with("https://kohaku.example.org/admin/reset/"));
    let form = harness.get(MAIN_HOST, path_of(&link)).await;
    assert!(body_text(form).await.contains("Choose a new password"));
    let (status, page, cookies) =
        set_password(&harness, path_of(&link), "the attacker password").await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("Password changed"));
    assert!(cookies.is_empty(), "no session is created");
    assert_eq!(
        harness.get_as(&session, "/admin").await.status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(harness.count("SELECT count(*) FROM known_devices").await, 0);
    let response = harness
        .login_as(
            &mut Browser::new(),
            "m@example.org",
            "the attacker password",
            "",
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK, "no code, no session");
    let db = Arc::clone(&harness.app.db);
    let after: String = db
        .read(move |c| c.query_row(state, [], |r| r.get(0)))
        .await
        .unwrap();
    let expected = before.replacen(" 3 ", " 4 ", 1);
    assert_eq!(
        after, expected,
        "TOTP, codes and lock state unchanged but for the failed login"
    );
    assert_eq!(
        harness
            .count("SELECT count(*) FROM audit_log WHERE action = 'user.password_reset' AND actor_label = 'maintainer' AND actor_id = 1")
            .await,
        1
    );
    assert_eq!(
        harness
            .count("SELECT count(*) FROM outbox WHERE kind = 'account_password_changed'")
            .await,
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn reset_link_replayed_or_raced() {
    let harness = Arc::new(Harness::new());
    let account = harness.account("m@example.org", "maintainer", false).await;
    ask(&harness, "m@example.org").await;
    let path = path_of(&mailed_link(&harness).await).to_owned();
    let tasks: Vec<_> = ["first new password", "second new password"]
        .into_iter()
        .map(|password| {
            let (harness, path) = (Arc::clone(&harness), path.clone());
            tokio::spawn(async move { set_password(&harness, &path, password).await.1 })
        })
        .collect();
    let mut done = 0;
    for task in tasks {
        let page = task.await.unwrap();
        if page.contains("Password changed") {
            done += 1;
        } else {
            assert!(page.contains("Link invalid or expired"));
        }
    }
    assert_eq!(done, 1, "exactly one parallel post changes the password");
    let hash = password_hash(&harness, account.id).await;
    let (_, page, _) = set_password(&harness, &path, "third new password").await;
    assert!(page.contains("Link invalid or expired"));
    ask(&harness, "m@example.org").await;
    let expired = path_of(&mailed_link(&harness).await).to_owned();
    harness.advance(60 * 60);
    let (_, page, _) = set_password(&harness, &expired, "fourth new password").await;
    assert!(page.contains("Link invalid or expired"), "after one hour");
    ask(&harness, "m@example.org").await;
    let replaced = path_of(&mailed_link(&harness).await).to_owned();
    ask(&harness, "m@example.org").await;
    let (_, page, _) = set_password(&harness, &replaced, "fifth new password").await;
    assert!(
        page.contains("Link invalid or expired"),
        "a newer link replaced it"
    );
    assert_eq!(password_hash(&harness, account.id).await, hash);
    let (status, page, _) =
        set_password(&harness, "/admin/reset/short", "sixth new password").await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("Link invalid or expired"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn get_never_consumes() {
    let harness = Harness::new();
    let account = harness.account("m@example.org", "maintainer", false).await;
    ask(&harness, "m@example.org").await;
    let link = mailed_link(&harness).await;
    let hash = password_hash(&harness, account.id).await;
    for _ in 0..10 {
        for method in ["GET", "HEAD"] {
            let response = harness
                .send(
                    request(method, MAIN_HOST, path_of(&link))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await;
            assert_eq!(response.status(), StatusCode::OK);
        }
    }
    assert_eq!(
        harness
            .count("SELECT count(*) FROM tokens WHERE used_at IS NULL")
            .await,
        1
    );
    assert_eq!(password_hash(&harness, account.id).await, hash);
    let (_, page, _) = set_password(&harness, path_of(&link), "a working new password").await;
    assert!(page.contains("Password changed"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn given_up_reset_mail_invalidates_its_link() {
    let harness = Harness::new();
    harness.account("m@example.org", "maintainer", false).await;
    ask(&harness, "m@example.org").await;
    let link = mailed_link(&harness).await;
    harness
        .app
        .db
        .write(|tx| {
            let id: i64 = tx.query_row("SELECT id FROM outbox", [], |r| r.get(0))?;
            give_up(tx, KINDS, id)
        })
        .await
        .unwrap();
    assert_eq!(
        harness.count("SELECT count(*) FROM outbox").await,
        0,
        "token mail is deleted"
    );
    let page = body_text(harness.get(MAIN_HOST, path_of(&link)).await).await;
    assert!(page.contains("Link invalid or expired"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn links_come_from_the_configured_base_url() {
    let harness = Harness::new();
    harness.account("m@example.org", "maintainer", false).await;
    let response = harness
        .send(
            request("POST", "KOHAKU.example.org:8443", "/admin/reset")
                .header("origin", "https://kohaku.example.org")
                .header("x-forwarded-host", "evil.example")
                .header("forwarded", "host=evil.example")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("email=m%40example.org"))
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        mailed_link(&harness)
            .await
            .starts_with("https://kohaku.example.org/admin/reset/")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn malformed_addresses_get_422() {
    let harness = Harness::new();
    for email in [
        "",
        "no-at-sign",
        "a b@example.org",
        "victim@example.com\r\nBcc: x@example.net",
    ] {
        let (status, _) = ask(&harness, email).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{email:?}");
    }
    assert_eq!(harness.count("SELECT count(*) FROM outbox").await, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn attacker_mail_bombs_one_mailbox_from_many_networks() {
    let capture = LogCapture::default();
    let _guard = capture.install();
    let harness = Harness::new();
    harness
        .account("victim@example.org", "maintainer", false)
        .await;
    let mut statuses = Vec::new();
    for (n, email) in [
        "victim@example.org",
        "Victim+a@Example.org",
        " victim+b@example.org",
        "VICTIM@example.org",
    ]
    .into_iter()
    .enumerate()
    {
        let peer = format!("[2001:db8:{}::1]:40000", n + 1);
        let request = from_peer(
            form_request("/admin/reset", &Browser::new(), &[("email", email)]),
            &peer,
        );
        statuses.push(harness.send(request).await.status());
    }
    assert_eq!(
        statuses,
        [
            StatusCode::OK,
            StatusCode::OK,
            StatusCode::OK,
            StatusCode::TOO_MANY_REQUESTS
        ]
    );
    assert!(
        !capture.text().to_lowercase().contains("victim"),
        "{}",
        capture.text()
    );
    let rows = harness
        .count("SELECT count(*) FROM outbox WHERE address LIKE '%victim%' OR body LIKE '%victim%'")
        .await;
    assert_eq!(rows, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mail_leaks_nothing() {
    let harness = Harness::new();
    let account = harness.account("m@example.org", "maintainer", false).await;
    let browser = harness.sign_in(&account).await;
    let hostile = "<script>alert(1)</script>\r\nBcc: list@example.net";
    for n in 0..10 {
        let email = if n % 2 == 0 { "m@example.org" } else { hostile };
        let request = from_peer(
            form_request(
                "/admin/login",
                &Browser::new(),
                &[("email", email), ("password", hostile), ("code", "123456")],
            ),
            "203.0.113.99:40000",
        );
        harness.send(request).await;
        if n % 2 == 1 {
            // Failures of the hostile address count for no account; make them count.
            harness
                .login_as(&mut Browser::new(), "m@example.org", hostile, "")
                .await;
        }
    }
    harness.advance(15 * 60);
    harness
        .post_as(
            &browser,
            "/admin/account/password",
            &[
                ("current_password", PASSWORD),
                ("new_password", "another password here"),
                ("new_password_again", "another password here"),
            ],
        )
        .await;
    ask(&harness, "m@example.org").await;
    let kinds = harness
        .count(
            "SELECT count(DISTINCT kind) FROM outbox WHERE kind IN
                 ('account_lockout', 'account_password_changed', 'account_reset_link')",
        )
        .await;
    assert_eq!(kinds, 3, "all three mails were queued");
    let texts: Vec<String> = harness
        .app
        .db
        .read(|c| {
            c.prepare("SELECT subject || char(10) || body FROM outbox")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .await
        .unwrap();
    for text in &texts {
        for forbidden in [
            "<script>",
            "\r",
            "Bcc",
            PASSWORD,
            "another password",
            "123456",
            "203.0.113",
            "198.18.",
            "m@example.org",
        ] {
            assert!(!text.contains(forbidden), "{forbidden} in {text}");
        }
    }
}

// Account commands

fn run(harness: &Harness, command: Command, secret_only: bool) -> (Result<(), String>, String) {
    let mut vars = vec![("KOHAKU_SECRET", TEST_SECRET)];
    if !secret_only {
        vars.push(("KOHAKU_BASE_URL", "https://kohaku.example.org"));
    }
    let lookup = move |name: &str| {
        vars.iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| OsString::from(v))
    };
    let mut stdout = Vec::new();
    let result = execute(
        command,
        &lookup,
        &harness.data,
        &mut std::io::empty(),
        &mut stdout,
    )
    .map_err(|e| e.to_string());
    (result, String::from_utf8(stdout).unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn admin_recovers_from_a_lock_or_a_forgotten_password() {
    let harness = Harness::new();
    let admin = harness.account("admin@example.org", "admin", true).await;
    for _ in 0..10 {
        harness
            .login_as(
                &mut Browser::new(),
                "admin@example.org",
                "wrong horse battery staple",
                "",
            )
            .await;
    }
    let unlock = Command::AdminUnlock("Admin@Example.org".to_owned());
    let (result, stdout) = tokio::task::block_in_place(|| run(&harness, unlock, true));
    assert_eq!(result, Ok(()));
    assert_eq!(stdout, "");
    let response = harness
        .login_as(
            &mut Browser::new(),
            "admin@example.org",
            PASSWORD,
            &harness.code(&admin, 0),
        )
        .await;
    assert_eq!(
        response.status(),
        StatusCode::SEE_OTHER,
        "no device cookie needed"
    );
    let reset = Command::AdminResetPassword("admin@example.org".to_owned());
    let (result, stdout) = tokio::task::block_in_place(|| run(&harness, reset, false));
    assert_eq!(result, Ok(()));
    assert_eq!(stdout.lines().count(), 1);
    assert!(
        stdout.starts_with("https://kohaku.example.org/admin/reset/") && stdout.ends_with('\n')
    );
    assert_eq!(
        harness
            .count("SELECT count(*) FROM outbox WHERE kind = 'account_reset_link'")
            .await,
        0
    );
    let (_, page, _) = set_password(
        &harness,
        path_of(stdout.trim_end()),
        "the admin's new password",
    )
    .await;
    assert!(page.contains("Password changed"));
    let audit: Vec<String> = harness
        .app
        .db
        .read(|c| {
            c.prepare("SELECT actor_label || ' ' || coalesce(actor_id, '-') || ' ' || action || ' ' || target_id FROM audit_log ORDER BY id")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
        .await
        .unwrap();
    assert_eq!(
        audit,
        [
            "cli - user.unlock 1",
            "cli - user.reset_link 1",
            "admin 1 user.password_reset 1"
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn commands_need_an_account() {
    let harness = Harness::new();
    for command in [
        Command::AdminUnlock("nobody@example.org".to_owned()),
        Command::AdminResetPassword("nobody@example.org".to_owned()),
    ] {
        let (result, stdout) = tokio::task::block_in_place(|| run(&harness, command, false));
        let message = result.unwrap_err();
        assert_eq!(message, "no account has that email address");
        assert_eq!(stdout, "");
    }
    assert_eq!(harness.count("SELECT count(*) FROM audit_log").await, 0);
    let (result, stdout) = tokio::task::block_in_place(|| {
        run(
            &harness,
            Command::AdminResetPassword("a@b.test".to_owned()),
            true,
        )
    });
    assert!(result.unwrap_err().contains("KOHAKU_BASE_URL"));
    assert_eq!(stdout, "");
}

#[test]
fn commands_refuse_a_missing_or_foreign_database() {
    let (_dir, data) = data_dir();
    let lookup = |name: &str| (name == "KOHAKU_SECRET").then(|| OsString::from(TEST_SECRET));
    let unlock = || Command::AdminUnlock("a@b.test".to_owned());
    let error = execute(
        unlock(),
        &lookup,
        &data,
        &mut std::io::empty(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("start `kohaku serve` first"),
        "{error}"
    );
    assert!(!data.database().exists(), "nothing is created");
    let conn = kohaku::db::migrate::open_and_prepare(
        &data,
        &other_secret(),
        kohaku::db::migrate::MIGRATIONS,
        1_800_000_000,
    )
    .unwrap();
    drop(conn);
    let error = execute(
        unlock(),
        &lookup,
        &data,
        &mut std::io::empty(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("KOHAKU_SECRET does not match"),
        "{error}"
    );
}
