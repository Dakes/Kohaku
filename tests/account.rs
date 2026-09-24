//! Sessions, CSRF, two-factor authentication and the account page (admin-auth:
//! Sessions, CSRF tokens, Two-factor authentication, Re-authentication for sensitive
//! account changes; change admin-auth D4, D8).

mod support;

use std::sync::Arc;

use axum::http::StatusCode;
use kohaku::auth::totp::{Seed, code, key_uri, qr_squares, step_at};
use kohaku::db::backup::backup_to_file;
use kohaku::keys::sha256;
use rusqlite::types::Value;
use support::*;

const ADMIN: &str = "admin@example.org";

async fn scalar<T>(harness: &Harness, sql: &'static str, id: i64) -> T
where
    T: rusqlite::types::FromSql + Send + 'static,
{
    let db = Arc::clone(&harness.app.db);
    db.read(move |c| c.query_row(sql, [id], |r| r.get::<_, T>(0)))
        .await
        .unwrap()
}

async fn signed_in(harness: &Harness, browser: &Browser) -> bool {
    match harness.get_as(browser, "/admin").await.status() {
        StatusCode::OK => true,
        StatusCode::SEE_OTHER => false,
        other => panic!("GET /admin: {other}"),
    }
}

/// Every byte of a fresh backup of the harness database.
async fn backup_bytes(harness: &Harness) -> Vec<u8> {
    let path = harness.data.root().join("scan.db");
    let data = harness.data.clone();
    let target = path.clone();
    tokio::task::spawn_blocking(move || backup_to_file(&data, &secret(), &target).unwrap())
        .await
        .unwrap();
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    bytes
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

// Sessions

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn attacker_fixes_or_replays_a_session() {
    let harness = Harness::new();
    let account = harness.account("m@example.org", "maintainer", false).await;
    let planted = "P".repeat(43);
    let mut browser = Browser {
        session: Some(planted.clone()),
        ..Browser::new()
    };
    let response = harness
        .login_as(&mut browser, &account.email, PASSWORD, "")
        .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let issued = browser.session.clone().unwrap();
    assert_ne!(issued, planted);
    let planted_browser = Browser {
        session: Some(planted),
        ..Browser::new()
    };
    assert!(!signed_in(&harness, &planted_browser).await);
    assert!(signed_in(&harness, &browser).await);
    let stored: Vec<u8> = scalar(
        &harness,
        "SELECT token_hash FROM sessions WHERE user_id = ?1",
        account.id,
    )
    .await;
    assert_eq!(stored, sha256(issued.as_bytes()));
    let backup = backup_bytes(&harness).await;
    assert!(
        !contains(&backup, issued.as_bytes()),
        "the backup holds no usable token"
    );
    assert!(!contains(
        &backup,
        browser.device.as_ref().unwrap().as_bytes()
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn idle_and_absolute_expiry() {
    let harness = Harness::new();
    let account = harness.account("m@example.org", "maintainer", false).await;
    let idle = harness.sign_in(&account).await;
    harness.advance(12 * 60 * 60 - 60);
    assert!(signed_in(&harness, &idle).await, "used 11 h 59 min ago");
    harness.advance(12 * 60 * 60 + 5 * 60);
    assert!(!signed_in(&harness, &idle).await, "unused for 12 h 5 min");
    let busy = harness.sign_in(&account).await;
    for hour in 1..7 * 24 {
        harness.advance(60 * 60);
        assert!(signed_in(&harness, &busy).await, "hour {hour}");
    }
    harness.advance(60 * 60);
    assert!(!signed_in(&harness, &busy).await, "7 days after it began");
    assert!(!signed_in(&harness, &busy).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn use_is_recorded_at_most_every_five_minutes() {
    let harness = Harness::new();
    let account = harness.account("m@example.org", "maintainer", false).await;
    let browser = harness.sign_in(&account).await;
    let seen = || {
        scalar::<i64>(
            &harness,
            "SELECT last_seen FROM sessions WHERE user_id = ?1",
            account.id,
        )
    };
    let first = seen().await;
    harness.advance(4 * 60);
    signed_in(&harness, &browser).await;
    assert_eq!(seen().await, first);
    harness.advance(60);
    signed_in(&harness, &browser).await;
    assert_eq!(seen().await, harness.now());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn attacker_enumerates_admin_paths() {
    let harness = Harness::new();
    let mut answers = Vec::new();
    for path in [
        "/admin",
        "/admin/account",
        "/admin/does-not-exist",
        "/admin/users",
        "/admin/",
    ] {
        let response = harness.get(MAIN_HOST, path).await;
        let location = header_values(&response, "location");
        answers.push((response.status(), location, body_text(response).await));
    }
    assert_eq!(answers[0].0, StatusCode::SEE_OTHER);
    assert_eq!(answers[0].1, ["/admin/login"]);
    for answer in &answers[1..] {
        assert_eq!(answer, &answers[0]);
    }
    let response = harness
        .send(form_request(
            "/admin/account/password",
            &Browser::new(),
            &[("csrf", "x")],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let not_found = body_text(harness.get(MAIN_HOST, "/does-not-exist").await).await;
    for path in ["/p/demo", "/p/demo/api/v1/reports"] {
        let response = harness.get(MAIN_HOST, path).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(body_text(response).await, not_found);
    }
    // Signed in, an unknown admin path is 404.
    let account = harness.account("m@example.org", "maintainer", false).await;
    let browser = harness.sign_in(&account).await;
    assert_eq!(
        harness.get_as(&browser, "/admin/users").await.status(),
        StatusCode::NOT_FOUND
    );
    let response = harness.post_as(&browser, "/admin/users", &[]).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sessions_end_with_their_account_state() {
    let harness = Harness::new();
    let disabled = harness.account("a@example.org", "maintainer", false).await;
    let passwordless = harness.account("b@example.org", "maintainer", false).await;
    let a = harness.sign_in(&disabled).await;
    let b = harness.sign_in(&passwordless).await;
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
    assert!(!signed_in(&harness, &a).await);
    assert!(!signed_in(&harness, &b).await);
}

// CSRF

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn attacker_forges_a_same_site_form_post() {
    let harness = Harness::new();
    let account = harness.account("m@example.org", "maintainer", false).await;
    let victim = harness.sign_in(&account).await;
    let other = harness.sign_in(&account).await;
    let hash_before: String = scalar(
        &harness,
        "SELECT password_hash FROM users WHERE id = ?1",
        account.id,
    )
    .await;
    let mut altered = victim.csrf.clone().into_bytes();
    altered[10] = if altered[10] == b'x' { b'y' } else { b'x' };
    let altered = String::from_utf8(altered).unwrap();
    let fields = [
        ("current_password", PASSWORD),
        ("new_password", "a new password for this"),
        ("new_password_again", "a new password for this"),
    ];
    for csrf in [None, Some(other.csrf.as_str()), Some(altered.as_str())] {
        let mut all: Vec<(&str, &str)> = csrf.map(|c| ("csrf", c)).into_iter().collect();
        all.extend_from_slice(&fields);
        let response = harness
            .send(form_request("/admin/account/password", &victim, &all))
            .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{csrf:?}");
    }
    let hash_after: String = scalar(
        &harness,
        "SELECT password_hash FROM users WHERE id = ?1",
        account.id,
    )
    .await;
    assert_eq!(hash_before, hash_after);
    assert!(signed_in(&harness, &victim).await);
    assert_eq!(
        harness.count("SELECT count(*) FROM audit_log").await,
        0,
        "nothing changed"
    );
}

// Two-factor authentication

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replayed_code() {
    let harness = Harness::new();
    let admin = harness.account(ADMIN, "admin", true).await;
    let code = harness.code(&admin, 0);
    let mut first = Browser::new();
    let response = harness.login_as(&mut first, ADMIN, PASSWORD, &code).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let mut second = Browser::new();
    let response = harness.login_as(&mut second, ADMIN, PASSWORD, &code).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(second.session.is_none());
    // An earlier step's code is replay too, once a later one was accepted.
    let earlier = harness.code(&admin, -1);
    let response = harness
        .login_as(&mut Browser::new(), ADMIN, PASSWORD, &earlier)
        .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn codes_race_in_parallel() {
    let harness = Arc::new(Harness::new());
    let admin = harness.account(ADMIN, "admin", true).await;
    harness
        .exec(
            "INSERT INTO recovery_codes (user_id, code_hash) VALUES (?1, ?2)",
            vec![
                Value::Integer(admin.id),
                Value::Blob(sha256(b"ABCDEFGHIJKLMNOP").to_vec()),
            ],
        )
        .await;
    for code in [harness.code(&admin, 0), "ABCD-EFGH-IJKL-MNOP".to_owned()] {
        let tasks: Vec<_> = (0..2)
            .map(|_| {
                let (harness, code) = (Arc::clone(&harness), code.clone());
                tokio::spawn(async move {
                    let mut browser = Browser::new();
                    harness
                        .login_as(&mut browser, ADMIN, PASSWORD, &code)
                        .await
                        .status()
                })
            })
            .collect();
        let mut statuses = Vec::new();
        for task in tasks {
            statuses.push(task.await.unwrap());
        }
        statuses.sort();
        assert_eq!(statuses, [StatusCode::OK, StatusCode::SEE_OTHER], "{code}");
    }
}

/// The path data of the page's QR code.
fn qr_path(page: &str) -> &str {
    let start = page.find("<path fill=\"#000000\" d=\"").unwrap() + 24;
    &page[start..start + page[start..].find('"').unwrap()]
}

fn seed_of(page: &str) -> &str {
    let start = page.find("<code class=\"secret\">").unwrap() + 21;
    &page[start..start + 32]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scanning_the_enrollment_qr_code() {
    let harness = Harness::new();
    let account = harness.account("m@example.org", "maintainer", false).await;
    let browser = harness.sign_in(&account).await;
    let other = harness.sign_in(&account).await;
    let response = harness
        .post_as(
            &browser,
            "/admin/account/totp",
            &[("current_password", PASSWORD)],
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header_values(&response, "content-security-policy"),
        [kohaku::http::headers::CSP]
    );
    let page = body_text(response).await;
    for forbidden in ["data:", "style=", "<img", "<script src=\"http", "<iframe"] {
        assert!(!page.contains(forbidden), "{forbidden}");
    }
    let nonce: Vec<u8> = scalar(
        &harness,
        "SELECT pending_totp_nonce FROM sessions WHERE user_id = ?1 AND pending_totp_nonce IS NOT NULL",
        account.id,
    )
    .await;
    let seed = Seed::derive(&secret(), account.id, &nonce);
    assert_eq!(seed_of(&page), seed.base32());
    let uri = key_uri(MAIN_HOST, &account.email, &seed);
    let squares = qr_squares(&uri);
    let expected: String = squares
        .dark
        .iter()
        .map(|(x, y)| format!("M{x} {y}h1v1h-1z"))
        .collect();
    assert_eq!(qr_path(&page), expected, "the QR code is the key URI");
    assert!(page.contains(&format!("viewBox=\"0 0 {0} {0}\"", squares.size)));
    assert!(page.contains("<rect width=\"") && page.contains("fill=\"#ffffff\""));
    // Not enrolled until a code is typed back; a wrong code shows the page again.
    assert_eq!(
        scalar::<Option<Vec<u8>>>(
            &harness,
            "SELECT totp_nonce FROM users WHERE id = ?1",
            account.id
        )
        .await,
        None
    );
    let wrong = if code(&seed, step_at(harness.now())) == "000000" {
        "111111"
    } else {
        "000000"
    };
    let response = harness
        .post_as(&browser, "/admin/account/totp/confirm", &[("code", wrong)])
        .await;
    let retry = body_text(response).await;
    assert!(retry.contains("That code is not right."));
    assert_eq!(qr_path(&retry), expected);
    let current = code(&seed, step_at(harness.now()));
    let response = harness
        .post_as(
            &browser,
            "/admin/account/totp/confirm",
            &[("code", &current)],
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let codes_page = body_text(response).await;
    assert_eq!(codes_page.matches("<li><code>").count(), 10);
    let stored: Option<Vec<u8>> = scalar(
        &harness,
        "SELECT totp_nonce FROM users WHERE id = ?1",
        account.id,
    )
    .await;
    assert_eq!(stored, Some(nonce));
    assert!(
        !signed_in(&harness, &other).await,
        "enrollment ends other sessions"
    );
    assert!(signed_in(&harness, &browser).await);
    assert_eq!(harness.count("SELECT count(*) FROM known_devices").await, 0);
    assert_eq!(
        harness.count("SELECT count(*) FROM recovery_codes").await,
        10
    );
    // The seed appears on no other page; the codes on no later page.
    let first_code = &codes_page[codes_page.find("<li><code>").unwrap() + 10..][..19];
    for path in ["/admin", "/admin/account"] {
        let page = body_text(harness.get_as(&browser, path).await).await;
        assert!(!page.contains(&seed.base32()), "{path}");
        assert!(!page.contains(first_code), "{path}");
    }
    assert!(!codes_page.contains(&seed.base32()));
    // The new seed signs in; the step just used does not again.
    harness.advance(30);
    let mut fresh = Browser::new();
    let next = code(&seed, step_at(harness.now()));
    let response = harness
        .login_as(&mut fresh, &account.email, PASSWORD, &next)
        .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let audit: String = scalar(
        &harness,
        "SELECT action || ' ' || actor_label || ' ' || actor_id || ' ' || target_id FROM audit_log WHERE id = ?1",
        1,
    )
    .await;
    assert_eq!(
        audit,
        format!("user.totp_enable maintainer {0} {0}", account.id)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn seed_is_not_in_the_data_directory() {
    let harness = Harness::new();
    let admin = harness.account(ADMIN, "admin", true).await;
    let browser = harness.sign_in(&admin).await;
    let seed = Seed::derive(&secret(), admin.id, &admin.nonce.unwrap());
    let response = harness
        .post_as(
            &browser,
            "/admin/account/recovery-codes",
            &[
                ("current_password", PASSWORD),
                ("code", &harness.code(&admin, 0)),
            ],
        )
        .await;
    let page = body_text(response).await;
    let codes: Vec<String> = page
        .match_indices("<li><code>")
        .map(|(i, _)| page[i + 10..i + 29].replace('-', ""))
        .collect();
    assert_eq!(codes.len(), 10);
    let mut files = vec![backup_bytes(&harness).await];
    for name in ["kohaku.db", "kohaku.db-wal"] {
        files.push(std::fs::read(harness.data.root().join(name)).unwrap_or_default());
    }
    let base32 = seed.base32();
    for bytes in &files {
        assert!(!contains(bytes, base32.as_bytes()));
        assert!(!contains(bytes, PASSWORD.as_bytes()));
        for code in &codes {
            assert!(
                !contains(bytes, code.as_bytes()),
                "recovery code in the data"
            );
        }
    }
    // The password is stored only as an argon2id PHC string with the fixed parameters.
    let hash: String = scalar(
        &harness,
        "SELECT password_hash FROM users WHERE id = ?1",
        admin.id,
    )
    .await;
    assert!(
        hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
        "{hash}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn second_admin_refused() {
    let harness = Harness::new();
    harness.account(ADMIN, "admin", false).await;
    let maintainer = harness.account("m@example.org", "maintainer", false).await;
    let db = Arc::clone(&harness.app.db);
    let insert = db
        .write(|tx| {
            tx.execute(
                "INSERT INTO users (email, role, created_at) VALUES ('b@example.org', 'admin', 0)",
                [],
            )
        })
        .await;
    assert!(insert.is_err());
    let id = maintainer.id;
    let update = db
        .write(move |tx| tx.execute("UPDATE users SET role = 'admin' WHERE id = ?1", [id]))
        .await;
    assert!(update.is_err());
}

// Re-authentication

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stolen_session_cannot_take_over_the_account() {
    let harness = Harness::new();
    let account = harness.account("m@example.org", "maintainer", true).await;
    harness
        .exec(
            "INSERT INTO recovery_codes (user_id, code_hash) VALUES (?1, ?2)",
            vec![
                Value::Integer(account.id),
                Value::Blob(sha256(b"ABCDEFGHIJKLMNOP").to_vec()),
            ],
        )
        .await;
    let victim = harness.sign_in(&account).await;
    let attacker = Browser {
        session: victim.session.clone(),
        csrf: victim.csrf.clone(),
        ..Browser::new()
    };
    let state = || async {
        let db = Arc::clone(&harness.app.db);
        let id = account.id;
        db.read(move |c| {
            c.query_row(
                "SELECT password_hash, totp_nonce, (SELECT group_concat(hex(code_hash)) FROM recovery_codes)
                 FROM users WHERE id = ?1",
                [id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?, r.get::<_, String>(2)?)),
            )
        })
        .await
        .unwrap()
    };
    let before = state().await;
    let new = [
        ("new_password", "attacker password 1"),
        ("new_password_again", "attacker password 1"),
    ];
    let attempts: Vec<(&str, Vec<(&str, &str)>)> = vec![
        ("/admin/account/password", new.to_vec()),
        ("/admin/account/totp/disable", vec![]),
        ("/admin/account/recovery-codes", vec![]),
        ("/admin/account/totp", vec![]),
        (
            "/admin/account/password",
            [new.as_slice(), &[("current_password", PASSWORD)]].concat(),
        ),
        (
            "/admin/account/totp/disable",
            vec![("current_password", PASSWORD)],
        ),
        (
            "/admin/account/recovery-codes",
            vec![("current_password", PASSWORD)],
        ),
    ];
    for (n, (path, fields)) in attempts.iter().enumerate() {
        let response = harness.post_as(&attacker, path, fields).await;
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        let page = body_text(response).await;
        assert!(page.contains("That did not work."), "{path}");
        let failures: i64 = scalar(
            &harness,
            "SELECT failed_logins FROM users WHERE id = ?1",
            account.id,
        )
        .await;
        assert_eq!(failures, n as i64 + 1, "{path} counts as a failure");
    }
    assert_eq!(state().await, before);
    assert!(signed_in(&harness, &victim).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn password_change() {
    let harness = Harness::new();
    let account = harness.account("m@example.org", "maintainer", false).await;
    let browser = harness.sign_in(&account).await;
    let other = harness.sign_in(&account).await;
    let token = harness.exec(
        "INSERT INTO tokens (purpose, token_hash, user_id, expires_at) VALUES ('reset', ?1, ?2, ?3)",
        vec![Value::Blob(vec![7; 32]), Value::Integer(account.id), Value::Integer(harness.now() + 3600)],
    );
    token.await;
    for (new, again, message) in [
        ("short", "short", "at least 12 characters"),
        ("a new password here", "a new password there", "differ"),
    ] {
        let response = harness
            .post_as(
                &browser,
                "/admin/account/password",
                &[
                    ("current_password", PASSWORD),
                    ("new_password", new),
                    ("new_password_again", again),
                ],
            )
            .await;
        assert!(body_text(response).await.contains(message), "{message}");
    }
    let response = harness
        .post_as(
            &browser,
            "/admin/account/password",
            &[
                ("current_password", PASSWORD),
                ("new_password", "a brand new password"),
                ("new_password_again", "a brand new password"),
            ],
        )
        .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(header_values(&response, "location"), ["/admin/login"]);
    assert!(header_values(&response, "set-cookie").is_empty());
    assert!(!signed_in(&harness, &browser).await);
    assert!(!signed_in(&harness, &other).await);
    assert_eq!(harness.count("SELECT count(*) FROM known_devices").await, 0);
    assert_eq!(
        harness
            .count("SELECT count(*) FROM tokens WHERE used_at IS NULL")
            .await,
        0
    );
    assert_eq!(
        harness
            .count("SELECT count(*) FROM outbox WHERE kind = 'account_password_changed' AND user_id = 1")
            .await,
        1
    );
    assert_eq!(
        harness
            .count("SELECT count(*) FROM audit_log WHERE action = 'user.password_change' AND target_id = 1")
            .await,
        1
    );
    let old = harness
        .login_as(&mut Browser::new(), &account.email, PASSWORD, "")
        .await;
    assert_eq!(old.status(), StatusCode::OK);
    let new = harness
        .login_as(
            &mut Browser::new(),
            &account.email,
            "a brand new password",
            "",
        )
        .await;
    assert_eq!(new.status(), StatusCode::SEE_OTHER);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn disable_and_regenerate() {
    let harness = Harness::new();
    let admin = harness.account(ADMIN, "admin", true).await;
    let browser = harness.sign_in(&admin).await;
    let response = harness
        .post_as(
            &browser,
            "/admin/account/totp/disable",
            &[
                ("current_password", PASSWORD),
                ("code", &harness.code(&admin, 0)),
            ],
        )
        .await;
    assert!(
        body_text(response)
            .await
            .contains("Your role requires two-factor login.")
    );
    let nonce: Option<Vec<u8>> = scalar(
        &harness,
        "SELECT totp_nonce FROM users WHERE id = ?1",
        admin.id,
    )
    .await;
    assert!(nonce.is_some(), "refused for the admin");

    let maintainer = harness.account("m@example.org", "maintainer", true).await;
    let browser = harness.sign_in(&maintainer).await;
    let response = harness
        .post_as(
            &browser,
            "/admin/account/recovery-codes",
            &[
                ("current_password", PASSWORD),
                ("code", &harness.code(&maintainer, 0)),
            ],
        )
        .await;
    let page = body_text(response).await;
    let first = page[page.find("<li><code>").unwrap() + 10..][..19].to_owned();
    assert!(page.contains("Your old recovery codes no longer work."));
    harness.advance(30);
    let response = harness
        .post_as(
            &browser,
            "/admin/account/recovery-codes",
            &[
                ("current_password", PASSWORD),
                ("code", &harness.code(&maintainer, 0)),
            ],
        )
        .await;
    assert!(!body_text(response).await.contains(&first));
    let replaced = harness
        .login_as(&mut Browser::new(), &maintainer.email, PASSWORD, &first)
        .await;
    assert_eq!(replaced.status(), StatusCode::OK, "old codes stop working");
    harness.advance(30);
    let response = harness
        .post_as(
            &browser,
            "/admin/account/totp/disable",
            &[
                ("current_password", PASSWORD),
                ("code", &harness.code(&maintainer, 0)),
            ],
        )
        .await;
    assert!(
        body_text(response)
            .await
            .contains("Two-factor login is off.")
    );
    let id = maintainer.id;
    assert_eq!(
        scalar::<Option<Vec<u8>>>(&harness, "SELECT totp_nonce FROM users WHERE id = ?1", id).await,
        None
    );
    assert_eq!(
        harness
            .count("SELECT count(*) FROM recovery_codes WHERE user_id = 2")
            .await,
        0
    );
    let actions: String = scalar(
        &harness,
        "SELECT group_concat(action, ' ') FROM audit_log WHERE target_id = ?1",
        id,
    )
    .await;
    assert_eq!(
        actions,
        "user.recovery_codes user.recovery_codes user.totp_disable"
    );
    let response = harness
        .login_as(&mut Browser::new(), &maintainer.email, PASSWORD, "")
        .await;
    assert_eq!(
        response.status(),
        StatusCode::SEE_OTHER,
        "password only now"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn end_other_sessions() {
    let harness = Harness::new();
    let account = harness.account("m@example.org", "maintainer", false).await;
    let current = harness.sign_in(&account).await;
    let others = [
        harness.sign_in(&account).await,
        harness.sign_in(&account).await,
    ];
    let response = harness
        .post_as(&current, "/admin/account/sessions/end-others", &[])
        .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(header_values(&response, "location"), ["/admin/account"]);
    assert!(signed_in(&harness, &current).await);
    for other in &others {
        assert!(!signed_in(&harness, other).await);
    }
    assert_eq!(
        harness
            .count("SELECT count(*) FROM audit_log WHERE action = 'user.sessions_end'")
            .await,
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pages_render_for_the_signed_in_account() {
    let harness = Harness::new();
    let admin = harness.account(ADMIN, "admin", true).await;
    let browser = harness.sign_in(&admin).await;
    let home = body_text(harness.get_as(&browser, "/admin").await).await;
    assert!(home.contains("admin@example.org (admin)"));
    assert!(home.contains("action=\"/admin/logout\""));
    let account = body_text(harness.get_as(&browser, "/admin/account").await).await;
    assert!(account.contains("0 unused recovery codes left"));
    assert!(account.contains("cannot be turned off"));
    // Sign out, password, new app, new codes and other sessions; no disable form.
    assert_eq!(
        account
            .matches(&format!("value=\"{}\"", browser.csrf))
            .count(),
        5
    );
}
