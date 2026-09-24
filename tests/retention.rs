//! The scheduler, retention and WAL checkpoints (data-storage: Retention job, Secure
//! deletion and WAL checkpoints; audit-log; change foundation D20).

mod support;

use std::fs;
use std::os::unix::fs::symlink;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use kohaku::db::migrate::{MIGRATIONS, Migration, open_and_prepare};
use kohaku::jobs::{checkpoint, remove_stale_tmp, retention};
use kohaku::mail::outbox::{KINDS, give_up};
use kohaku::time::now_unix;
use support::*;

const DAY: i64 = 86_400;
const HOUR: i64 = 3600;

async fn count(harness: &Harness, sql: &'static str) -> i64 {
    harness
        .app
        .db
        .read(move |c| c.query_row(sql, [], |r| r.get(0)))
        .await
        .unwrap()
}

async fn exec(harness: &Harness, sql: String) {
    harness
        .app
        .db
        .write(move |tx| tx.execute_batch(&sql))
        .await
        .unwrap();
}

fn audit_at(time: i64) -> String {
    format!(
        "INSERT INTO audit_log (actor_label, actor_id, action, target_type, target_id, time)
         VALUES ('maintainer', 7, 'sample.update', 'sample', 42, {time});"
    )
}

fn outbox_row(subject: &str, outcome: &str, queued: i64, expires: i64) -> String {
    format!(
        "INSERT INTO outbox (kind, address, subject, body, priority, token, placeholder,
             next_attempt_at, queued_at, expires_at, outcome)
         VALUES ('test', 'a@example.com', '{subject}', 'b', 'normal', 0, 0, {queued}, {queued}, {expires}, {outcome});"
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runs_at_startup_and_purges_only_old_audit_entries() {
    let harness = Harness::new();
    let now = now_unix();
    exec(
        &harness,
        audit_at(now - 366 * DAY) + &audit_at(now - 364 * DAY),
    )
    .await;
    let (stop, stopped) = tokio::sync::watch::channel(());
    let task = tokio::spawn(kohaku::jobs::run(
        Arc::clone(&harness.app),
        harness.data.clone(),
        KINDS,
        stopped,
    ));
    for _ in 0..100 {
        if count(&harness, "SELECT count(*) FROM audit_log").await == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        count(&harness, "SELECT count(*) FROM audit_log").await,
        1,
        "no waiting 60 minutes"
    );
    let kept: i64 = count(&harness, "SELECT time FROM audit_log").await;
    assert_eq!(kept, now - 364 * DAY);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(fs::metadata(harness.data.wal()).unwrap().len(), 0);
    // Stops on shutdown.
    let started = Instant::now();
    stop.send(()).unwrap();
    task.await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_run_is_logged_and_retried() {
    let harness = Harness::new();
    let now = now_unix();
    exec(&harness, audit_at(now - 366 * DAY)).await;
    // Another process holds the write lock past the busy timeout.
    let blocker = kohaku::db::open(&harness.data.database(), kohaku::db::Role::Command).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = Instant::now();
    retention(&harness.app.db, &harness.data, KINDS, now).await;
    assert!(started.elapsed() >= Duration::from_secs(5));
    assert_eq!(count(&harness, "SELECT count(*) FROM audit_log").await, 1);
    assert_eq!(
        harness.get(MAIN_HOST, "/").await.status(),
        axum::http::StatusCode::OK
    );
    blocker.execute_batch("ROLLBACK").unwrap();
    retention(&harness.app.db, &harness.data, KINDS, now).await;
    assert_eq!(count(&harness, "SELECT count(*) FROM audit_log").await, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn failure_log_holds_no_row_content() {
    let capture = LogCapture::default();
    let _guard = capture.install();
    let harness = Harness::new();
    let now = now_unix();
    exec(
        &harness,
        outbox_row("secret-subject", "'sent'", now, now + DAY),
    )
    .await;
    let blocker = kohaku::db::open(&harness.data.database(), kohaku::db::Role::Command).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    retention(&harness.app.db, &harness.data, KINDS, now).await;
    blocker.execute_batch("ROLLBACK").unwrap();
    let log = capture.text();
    assert!(log.contains("retention step outbox failed"), "{log}");
    assert!(
        !log.contains("secret-subject") && !log.contains("a@example.com"),
        "{log}"
    );
}

#[tokio::test]
async fn finished_and_expired_outbox_rows_are_purged() {
    let harness = Harness::new();
    let now = now_unix();
    let rows = [
        outbox_row("sent", "'sent'", now - HOUR, now + DAY),
        outbox_row("given-up", "'given_up'", now - HOUR, now + DAY),
        outbox_row("expired", "NULL", now - HOUR, now - 1),
        outbox_row("old", "NULL", now - 25 * HOUR, now + DAY),
        outbox_row("kept", "NULL", now - 23 * HOUR, now + DAY),
    ];
    exec(&harness, rows.concat()).await;
    retention(&harness.app.db, &harness.data, KINDS, now).await;
    let left: Vec<String> = harness
        .app
        .db
        .read(|c| {
            c.prepare("SELECT subject FROM outbox")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()
        })
        .await
        .unwrap();
    assert_eq!(left, ["kept"]);
}

fn file_aged(path: &std::path::Path, hours: u64) {
    fs::write(path, b"x").unwrap();
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(hours * 3600))
        .unwrap();
}

#[test]
fn only_stale_temporary_files_are_removed() {
    let (dir, data) = data_dir();
    let conn = open_and_prepare(&data, &secret(), MIGRATIONS, 1).unwrap();
    let backups = data.ensure_backups().unwrap();
    for folder in [dir.path().to_path_buf(), backups.clone()] {
        file_aged(&folder.join(".old.tmp"), 25);
        file_aged(&folder.join(".new.tmp"), 1);
    }
    file_aged(&backups.join("2026-09-23.db"), 30 * 24);
    file_aged(&backups.join("pre-migrate-v1-1790000000.db"), 30 * 24);
    fs::create_dir(backups.join("sub")).unwrap();
    file_aged(&backups.join("sub/.inside.tmp"), 30 * 24);
    let outside = TempDir::new();
    let target = outside.path().join("target.tmp");
    file_aged(&target, 30 * 24);
    symlink(&target, backups.join("old.tmp")).unwrap();
    file_aged(&dir.path().join("kohaku.lock"), 30 * 24);
    let db_before = fs::read(data.database()).unwrap();
    let now = SystemTime::now();
    remove_stale_tmp(dir.path(), now).unwrap();
    remove_stale_tmp(&backups, now).unwrap();
    for folder in [dir.path().to_path_buf(), backups.clone()] {
        assert!(!folder.join(".old.tmp").exists());
        assert!(folder.join(".new.tmp").exists());
    }
    for kept in [
        "2026-09-23.db",
        "pre-migrate-v1-1790000000.db",
        "sub/.inside.tmp",
        "old.tmp",
    ] {
        assert!(fs::symlink_metadata(backups.join(kept)).is_ok(), "{kept}");
    }
    assert!(target.exists());
    assert!(dir.path().join("kohaku.lock").exists());
    assert_eq!(fs::read(data.database()).unwrap(), db_before);
    drop(conn);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deleted_data_cannot_be_recovered_from_the_live_files() {
    let harness = Harness::new();
    let now = now_unix();
    exec(
        &harness,
        outbox_row("MARKER-5f3a9c", "NULL", now, now + DAY),
    )
    .await;
    assert!(checkpoint(&harness.app.db).await);
    exec(&harness, "DELETE FROM outbox".to_owned()).await;
    assert!(checkpoint(&harness.app.db).await);
    assert_eq!(fs::metadata(harness.data.wal()).unwrap().len(), 0);
    for path in [harness.data.database(), harness.data.wal()] {
        let bytes = fs::read(&path).unwrap();
        assert!(
            !bytes.windows(13).any(|w| w == b"MARKER-5f3a9c"),
            "{path:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocked_checkpoint_fails_no_request() {
    let harness = Harness::new();
    exec(&harness, outbox_row("x", "NULL", 1, 2)).await;
    let reader = kohaku::db::open(&harness.data.database(), kohaku::db::Role::Reader).unwrap();
    reader.execute_batch("BEGIN").unwrap();
    let _: i64 = reader
        .query_row("SELECT count(*) FROM outbox", [], |r| r.get(0))
        .unwrap();
    exec(&harness, "DELETE FROM outbox".to_owned()).await;
    assert!(!checkpoint(&harness.app.db).await);
    assert_eq!(
        harness.get(MAIN_HOST, "/").await.status(),
        axum::http::StatusCode::OK
    );
    reader.execute_batch("COMMIT").unwrap();
    assert!(checkpoint(&harness.app.db).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_public_writes_need_permits_and_failures_return_them() {
    let harness = Harness::new();
    let mut held: Vec<_> = (0..8)
        .map(|_| harness.app.permits.try_public_write().unwrap())
        .collect();
    assert!(harness.app.permits.try_public_write().is_none());
    let now = now_unix();
    exec(&harness, audit_at(now - 400 * DAY)).await;
    let started = Instant::now();
    retention(&harness.app.db, &harness.data, KINDS, now).await;
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(count(&harness, "SELECT count(*) FROM audit_log").await, 0);
    // A held write fails: its permit returns with it.
    let failed = held.pop().unwrap();
    let result: Result<(), rusqlite::Error> = harness
        .app
        .db
        .write(move |tx| {
            let _permit = failed;
            tx.execute_batch("INSERT INTO nowhere VALUES (1)")
        })
        .await;
    assert!(result.is_err());
    assert!(harness.app.permits.try_public_write().is_some());
}

#[tokio::test]
async fn read_only_commands_and_background_work_record_nothing() {
    let (_dir, data) = data_dir();
    let newer = [
        MIGRATIONS[0],
        Migration {
            version: 2,
            name: "0002_next",
            sql: "CREATE TABLE next (id INTEGER PRIMARY KEY) STRICT;",
        },
    ];
    let conn = open_and_prepare(&data, &secret(), MIGRATIONS, 1).unwrap();
    conn.execute_batch(&audit_at(now_unix())).unwrap();
    drop(conn);
    let entries = |data: &kohaku::db::DataDir| -> i64 {
        let conn = kohaku::db::open(&data.database(), kohaku::db::Role::Command).unwrap();
        conn.query_row("SELECT count(*) FROM audit_log", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(entries(&data), 1);
    // A migrating start, backups and the listing.
    let conn = open_and_prepare(&data, &secret(), &newer, 2).unwrap();
    let db = Arc::new(kohaku::db::Db::new(conn, &data.database()).unwrap());
    let mut out = Vec::new();
    kohaku::db::backup::backup_to_writer(&data, &secret(), &mut out).unwrap();
    let files = TempDir::new();
    kohaku::db::backup::backup_to_file(&data, &secret(), &files.path().join("b.db")).unwrap();
    kohaku::db::backup::list_backups(&data).unwrap();
    // Retention, and the outbox sending one row and giving up another.
    let now = now_unix();
    db.write(move |tx| {
        tx.execute_batch(
            &(outbox_row("a", "NULL", now, now + DAY) + &outbox_row("b", "NULL", now, now + DAY)),
        )?;
        kohaku::mail::outbox::record(tx, KINDS, 1, kohaku::mail::outbox::Outcome::Sent, now)?;
        give_up(tx, KINDS, 2)
    })
    .await
    .unwrap();
    retention(&db, &data, KINDS, now).await;
    assert_eq!(entries(&data), 1);
}

#[tokio::test]
async fn expired_sign_in_state_is_purged() {
    let harness = Harness::new();
    let now = now_unix();
    let mut sql = format!(
        "INSERT INTO users (id, email, role, created_at) VALUES (1, 'm@example.org', 'maintainer', {now});"
    );
    for (n, (created, seen)) in [
        (now - HOUR, now - 13 * HOUR),
        (now - 8 * DAY, now - HOUR),
        (now - HOUR, now - 60),
    ]
    .into_iter()
    .enumerate()
    {
        sql.push_str(&format!(
            "INSERT INTO sessions (id, token_hash, user_id, csrf_token, created_at, last_seen)
             VALUES ({}, randomblob(32), 1, randomblob(32), {created}, {seen});",
            n + 1
        ));
    }
    for (n, (used, expires)) in [
        (format!("{}", now - 60), now + HOUR),
        ("NULL".to_owned(), now - 1),
        ("NULL".to_owned(), now + HOUR),
    ]
    .into_iter()
    .enumerate()
    {
        sql.push_str(&format!(
            "INSERT INTO tokens (id, purpose, token_hash, user_id, expires_at, used_at)
             VALUES ({}, 'reset', randomblob(32), 1, {expires}, {used});",
            n + 1
        ));
    }
    for (n, created) in [now - 366 * DAY, now - 10 * DAY].into_iter().enumerate() {
        sql.push_str(&format!(
            "INSERT INTO known_devices (id, user_id, token_hash, created_at)
             VALUES ({}, 1, randomblob(32), {created});",
            n + 1
        ));
    }
    exec(&harness, sql).await;
    retention(&harness.app.db, &harness.data, KINDS, now).await;
    let ids = |table: &'static str| {
        let db = Arc::clone(&harness.app.db);
        async move {
            db.read(move |c| {
                c.prepare(&format!("SELECT id FROM {table} ORDER BY id"))?
                    .query_map([], |r| r.get::<_, i64>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            .unwrap()
        }
    };
    assert_eq!(ids("sessions").await, [3], "only the fresh session");
    assert_eq!(ids("tokens").await, [3], "only the valid token");
    assert_eq!(
        ids("known_devices").await,
        [2],
        "only the 10-day-old device cookie"
    );
}
