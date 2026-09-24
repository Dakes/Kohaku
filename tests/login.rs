//! Login, hashing permits and lockout (admin-auth: Login, Password hashing is bounded,
//! Lockout; change admin-auth D5–D7).

mod support;

use std::sync::atomic::Ordering;

use axum::http::StatusCode;
use kohaku::keys::sha256;
use rusqlite::types::Value;
use support::*;

const ADMIN: &str = "admin@example.org";

fn checks(harness: &Harness) -> u64 {
    harness.app.password_checks.load(Ordering::SeqCst)
}

async fn failed_logins(harness: &Harness, id: i64) -> i64 {
    let db = std::sync::Arc::clone(&harness.app.db);
    db.read(move |c| {
        c.query_row("SELECT failed_logins FROM users WHERE id = ?1", [id], |r| {
            r.get::<_, i64>(0)
        })
    })
    .await
    .unwrap()
}

async fn locked(harness: &Harness, id: i64) -> bool {
    let (db, now) = (std::sync::Arc::clone(&harness.app.db), harness.now());
    db.read(move |c| {
        c.query_row(
            "SELECT coalesce(locked_until > ?2, 0) FROM users WHERE id = ?1",
            [id, now],
            |r| r.get::<_, bool>(0),
        )
    })
    .await
    .unwrap()
}

/// A login from a new browser; its status, body and cookies.
async fn attempt(
    harness: &Harness,
    email: &str,
    password: &str,
    code: &str,
) -> (u16, String, Vec<String>) {
    let mut browser = Browser::new();
    let response = harness.login_as(&mut browser, email, password, code).await;
    let cookies = header_values(&response, "set-cookie");
    (
        response.status().as_u16(),
        body_text(response).await,
        cookies,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn attacker_probes_accounts() {
    let harness = Harness::new();
    harness.account(ADMIN, "admin", true).await;
    let disabled = harness
        .account("disabled@example.org", "maintainer", false)
        .await;
    let passwordless = harness
        .account("new@example.org", "maintainer", false)
        .await;
    let locked_account = harness
        .account("locked@example.org", "maintainer", false)
        .await;
    harness
        .account("real@example.org", "maintainer", false)
        .await;
    harness
        .exec(
            "UPDATE users SET disabled = 1 WHERE id = ?1",
            vec![Value::Integer(disabled.id)],
        )
        .await;
    harness
        .exec(
            "UPDATE users SET password_hash = NULL WHERE id = ?1",
            vec![Value::Integer(passwordless.id)],
        )
        .await;
    harness
        .exec(
            "UPDATE users SET locked_until = ?2 WHERE id = ?1",
            vec![
                Value::Integer(locked_account.id),
                Value::Integer(harness.now() + 900),
            ],
        )
        .await;
    let mut answers = Vec::new();
    for (email, password) in [
        ("unknown@example.org", PASSWORD),
        ("disabled@example.org", PASSWORD),
        ("new@example.org", PASSWORD),
        ("locked@example.org", PASSWORD),
        ("real@example.org", "wrong horse battery staple"),
    ] {
        let before = checks(&harness);
        answers.push(attempt(&harness, email, password, "").await);
        assert_eq!(checks(&harness), before + 1, "{email}: argon2id once");
    }
    let (status, body, cookies) = &answers[0];
    assert_eq!(*status, 200);
    assert!(body.contains("The sign-in failed."));
    assert!(cookies.is_empty());
    for answer in &answers[1..] {
        assert_eq!(answer, &answers[0]);
    }
    assert!(
        !answers[0].1.contains("example.org"),
        "no submitted value is shown"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn attacker_has_the_password_but_not_the_code() {
    let harness = Harness::new();
    let admin = harness.account(ADMIN, "admin", true).await;
    let recovery = "ABCDEFGHIJKLMNOP";
    harness
        .exec(
            "INSERT INTO recovery_codes (user_id, code_hash) VALUES (?1, ?2)",
            vec![
                Value::Integer(admin.id),
                Value::Blob(sha256(recovery.as_bytes()).to_vec()),
            ],
        )
        .await;
    let (status, ..) = attempt(&harness, ADMIN, PASSWORD, "abcd-efgh-ijkl-mnop").await;
    assert_eq!(status, 303, "an unused recovery code signs in");
    let sessions = harness.count("SELECT count(*) FROM sessions").await;
    let current: Vec<String> = (-1..=1).map(|o| harness.code(&admin, o)).collect();
    let wrong = (0..1_000_000)
        .map(|n| format!("{n:06}"))
        .find(|c| !current.contains(c))
        .unwrap();
    let old = harness.code(&admin, -2);
    for (n, code) in ["", wrong.as_str(), old.as_str(), "ABCD-EFGH-IJKL-MNOP"]
        .into_iter()
        .enumerate()
    {
        let (status, body, cookies) = attempt(&harness, ADMIN, PASSWORD, code).await;
        assert_eq!(status, 200, "{code:?}");
        assert!(body.contains("The sign-in failed."));
        assert!(cookies.is_empty());
        assert_eq!(
            failed_logins(&harness, admin.id).await,
            n as i64 + 1,
            "{code:?}"
        );
    }
    assert_eq!(
        harness.count("SELECT count(*) FROM sessions").await,
        sessions
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_redirect_target_from_the_request() {
    let harness = Harness::new();
    let account = harness.account("m@example.org", "maintainer", false).await;
    let browser = Browser::new();
    let mut request = form_request(
        "/admin/login?next=https://evil.example",
        &browser,
        &[
            ("email", &account.email),
            ("password", PASSWORD),
            ("next", "https://evil.example"),
        ],
    );
    request
        .headers_mut()
        .insert("referer", "https://evil.example/".parse().unwrap());
    let response = harness.send(request).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(header_values(&response, "location"), ["/admin"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unenrolled_admin_is_told_two_factor_is_required() {
    let harness = Harness::new();
    let admin = harness.account(ADMIN, "admin", false).await;
    attempt(&harness, ADMIN, "wrong horse battery staple", "").await;
    assert_eq!(failed_logins(&harness, admin.id).await, 1);
    let (status, body, cookies) = attempt(&harness, ADMIN, PASSWORD, "").await;
    assert_eq!(status, 200);
    assert!(body.contains("Two-factor login required"), "{body}");
    assert!(body.contains("setup link"));
    assert!(cookies.is_empty());
    assert_eq!(harness.count("SELECT count(*) FROM sessions").await, 0);
    assert_eq!(
        failed_logins(&harness, admin.id).await,
        1,
        "neither counted nor reset"
    );
    // A wrong password still gets the generic failure, not this page.
    let (_, body, _) = attempt(&harness, ADMIN, "wrong horse battery staple", "").await;
    assert!(body.contains("The sign-in failed."));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signed_in_login_page_goes_home() {
    let harness = Harness::new();
    let account = harness.account("m@example.org", "maintainer", false).await;
    let browser = harness.sign_in(&account).await;
    let response = harness.get_as(&browser, "/admin/login").await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(header_values(&response, "location"), ["/admin"]);
    let response = harness.get_as(&Browser::new(), "/admin/login").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(header_values(&response, "set-cookie").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn login_flood_cannot_starve_the_owner() {
    let harness = std::sync::Arc::new(Harness::new());
    let admin = harness.account(ADMIN, "admin", true).await;
    let owner = harness.sign_in(&admin).await;
    // The flood holds the general permit; 50 more requests wait for it.
    let held = harness.app.permits.hashing(false).await.unwrap();
    let flood: Vec<_> = (0..50)
        .map(|_| {
            let harness = std::sync::Arc::clone(&harness);
            tokio::spawn(async move {
                let started = std::time::Instant::now();
                let (status, ..) = attempt(&harness, ADMIN, "guess guess guess", "").await;
                (status, started.elapsed())
            })
        })
        .collect();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let mut browser = Browser {
        session: None,
        ..owner.clone()
    };
    let started = std::time::Instant::now();
    let code = harness.code(&admin, 0);
    let response = harness.login_as(&mut browser, ADMIN, PASSWORD, &code).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "the owner did not wait"
    );
    for task in flood {
        let (status, waited) = task.await.unwrap();
        assert_eq!(status, 503);
        assert!(waited < std::time::Duration::from_secs(3), "{waited:?}");
    }
    assert_eq!(
        failed_logins(&harness, admin.id).await,
        0,
        "503 is not a failure"
    );
    drop(held);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stranger_cannot_lock_the_owner_out() {
    let harness = Harness::new();
    let admin = harness.account(ADMIN, "admin", true).await;
    let mut owner = harness.sign_in(&admin).await;
    owner.session = None;
    for n in 1..=10 {
        attempt(&harness, ADMIN, "wrong horse battery staple", "").await;
        assert_eq!(locked(&harness, admin.id).await, n == 10, "after {n}");
    }
    let lockout_mails = "SELECT count(*) FROM outbox WHERE kind = 'account_lockout'";
    assert_eq!(harness.count(lockout_mails).await, 1);
    let (status, body, _) = attempt(&harness, ADMIN, PASSWORD, &harness.code(&admin, 0)).await;
    assert_eq!(status, 200);
    assert!(body.contains("The sign-in failed."));
    assert!(
        !body.to_lowercase().contains("lock"),
        "no response says locked"
    );
    let response = harness
        .login_as(&mut owner, ADMIN, PASSWORD, &harness.code(&admin, 0))
        .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        locked(&harness, admin.id).await,
        "success does not end the lock"
    );
    let mail: String = harness
        .app
        .db
        .read(|c| {
            c.query_row(
                "SELECT body FROM outbox WHERE kind = 'account_lockout'",
                [],
                |r| r.get(0),
            )
        })
        .await
        .unwrap();
    assert!(mail.contains("locked for 15 minutes"));
    assert!(mail.contains("10 failed"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lock_ends_and_restarts_cleanly() {
    let harness = Harness::new();
    let account = harness.account("m@example.org", "maintainer", false).await;
    for _ in 0..10 {
        attempt(&harness, &account.email, "wrong horse battery staple", "").await;
    }
    assert!(locked(&harness, account.id).await);
    // Attempts while locked neither count nor extend it.
    for _ in 0..5 {
        attempt(&harness, &account.email, "wrong horse battery staple", "").await;
    }
    assert_eq!(failed_logins(&harness, account.id).await, 0);
    harness.advance(15 * 60);
    assert!(!locked(&harness, account.id).await);
    for _ in 0..9 {
        attempt(&harness, &account.email, "wrong horse battery staple", "").await;
    }
    assert!(!locked(&harness, account.id).await);
    assert_eq!(failed_logins(&harness, account.id).await, 9);
    attempt(&harness, &account.email, "wrong horse battery staple", "").await;
    assert!(locked(&harness, account.id).await);
    let mails = harness
        .count("SELECT count(*) FROM outbox WHERE kind = 'account_lockout'")
        .await;
    assert_eq!(mails, 1, "no second lockout mail within 24 hours");
    // A day after the first mail, a new lock mails again.
    harness.advance(24 * 60 * 60);
    for _ in 0..10 {
        attempt(&harness, &account.email, "wrong horse battery staple", "").await;
    }
    let mails = harness
        .count("SELECT count(*) FROM outbox WHERE kind = 'account_lockout'")
        .await;
    assert_eq!(mails, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stolen_device_cookie_is_bounded() {
    let harness = Harness::new();
    let admin = harness.account(ADMIN, "admin", true).await;
    let stolen = harness.sign_in(&admin).await;
    // A new address each time, so only the device window limits the attacker.
    let attacker = |_: usize| Browser {
        device: stolen.device.clone(),
        ..Browser::new()
    };
    for n in 0..20 {
        let mut browser = attacker(n);
        let response = harness
            .login_as(&mut browser, ADMIN, "wrong horse battery staple", "")
            .await;
        assert_eq!(response.status(), 200);
        harness.advance(60);
    }
    assert!(
        !locked(&harness, admin.id).await,
        "device failures never lock"
    );
    for n in 20..25 {
        let mut browser = attacker(n);
        let code = harness.code(&admin, 0);
        let response = harness.login_as(&mut browser, ADMIN, PASSWORD, &code).await;
        assert_eq!(response.status(), 200, "attempt {}", n + 1);
    }
    // One hour after the first failure the window ends.
    harness.advance(3600 - 20 * 60);
    let mut browser = attacker(25);
    let code = harness.code(&admin, 0);
    let response = harness.login_as(&mut browser, ADMIN, PASSWORD, &code).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_failures_cannot_overshoot() {
    let harness = std::sync::Arc::new(Harness::new());
    let account = harness.account("m@example.org", "maintainer", false).await;
    let tasks: Vec<_> = (0..30)
        .map(|_| {
            let harness = std::sync::Arc::clone(&harness);
            tokio::spawn(async move {
                attempt(&harness, "m@example.org", "wrong horse battery staple", "")
                    .await
                    .0
            })
        })
        .collect();
    for task in tasks {
        assert_eq!(task.await.unwrap(), 200);
    }
    assert!(locked(&harness, account.id).await);
    assert_eq!(
        failed_logins(&harness, account.id).await,
        0,
        "reset when the lock started"
    );
    let mails = harness
        .count("SELECT count(*) FROM outbox WHERE kind = 'account_lockout'")
        .await;
    assert_eq!(mails, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn device_cookies_are_kept_ten_per_account() {
    let harness = Harness::new();
    let account = harness.account("m@example.org", "maintainer", false).await;
    let first = harness.sign_in(&account).await;
    for _ in 0..10 {
        harness.advance(1);
        harness.sign_in(&account).await;
    }
    assert_eq!(
        harness.count("SELECT count(*) FROM known_devices").await,
        10
    );
    let hash = sha256(first.device.as_ref().unwrap().as_bytes()).to_vec();
    let db = std::sync::Arc::clone(&harness.app.db);
    let present: bool = db
        .read(move |c| {
            c.query_row(
                "SELECT EXISTS (SELECT 1 FROM known_devices WHERE token_hash = ?1)",
                [hash],
                |r| r.get(0),
            )
        })
        .await
        .unwrap();
    assert!(!present, "the oldest device cookie went first");
    // A login carrying a known device cookie keeps it and sets no new one.
    let mut again = Browser {
        session: None,
        ..harness.sign_in(&account).await
    };
    let response = harness
        .login_as(&mut again, &account.email, PASSWORD, "")
        .await;
    let cookies = header_values(&response, "set-cookie");
    assert_eq!(cookies.len(), 1);
    assert!(cookies[0].starts_with("__Host-kohaku_session="));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cookies_only_from_login_and_logout() {
    let harness = Harness::new();
    harness.app.host_map.replace(
        kohaku::routing::hosts::HostMap::new(&harness.app.base_url)
            .with_project("bugs.example.net", kohaku::routing::hosts::ProjectId(1)),
    );
    let account = harness.account("m@example.org", "maintainer", false).await;
    let forged = "__Host-kohaku_session=forged";
    for route in kohaku::routing::table::table() {
        for host in [MAIN_HOST, "bugs.example.net", "evil.example"] {
            let response = harness
                .send(
                    request("GET", host, &route.example)
                        .header("cookie", forged)
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await;
            assert!(
                header_values(&response, "set-cookie").is_empty(),
                "{host}{}",
                route.example
            );
        }
    }
    let (_, _, cookies) = attempt(&harness, &account.email, "wrong horse battery staple", "").await;
    assert!(cookies.is_empty());
    let mut browser = Browser::new();
    let response = harness
        .login_as(&mut browser, &account.email, PASSWORD, "")
        .await;
    let cookies = header_values(&response, "set-cookie");
    assert_eq!(cookies.len(), 2);
    for cookie in &cookies {
        assert!(cookie.starts_with("__Host-kohaku_"), "{cookie}");
        for attribute in ["; Secure", "; HttpOnly", "; SameSite=Strict", "; Path=/"] {
            assert!(cookie.contains(attribute), "{cookie}");
        }
        assert!(!cookie.to_lowercase().contains("domain"), "{cookie}");
    }
    browser.csrf = csrf_of(&body_text(harness.get_as(&browser, "/admin").await).await);
    let response = harness.post_as(&browser, "/admin/logout", &[]).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(header_values(&response, "location"), ["/admin/login"]);
    assert_eq!(
        header_values(&response, "set-cookie"),
        ["__Host-kohaku_session=; Secure; HttpOnly; SameSite=Strict; Path=/; Max-Age=0"]
    );
    assert_eq!(harness.count("SELECT count(*) FROM sessions").await, 0);
    assert_eq!(
        harness.get_as(&browser, "/admin").await.status(),
        StatusCode::SEE_OTHER
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn oversize_fields_fail_like_any_other() {
    let harness = Harness::new();
    let account = harness.account("m@example.org", "maintainer", false).await;
    let long_password = "p".repeat(1025);
    let long_code = "1".repeat(33);
    let (status, body, _) = attempt(&harness, &account.email, &long_password, "").await;
    assert_eq!(status, 200);
    assert!(body.contains("The sign-in failed."));
    let (status, ..) = attempt(&harness, &account.email, PASSWORD, &long_code).await;
    assert_eq!(status, 200);
    assert_eq!(failed_logins(&harness, account.id).await, 2);
    let (status, ..) = attempt(&harness, &"e".repeat(300), PASSWORD, "").await;
    assert_eq!(status, 200);
}
