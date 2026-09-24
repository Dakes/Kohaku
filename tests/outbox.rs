//! The outbox and its worker with a fake mailer (mail-outbox; change foundation D17).

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kohaku::db::Db;
use kohaku::mail::outbox::{
    MailKind, NewMail, Outcome, Pick, Priority, Recipient, Wakeup, Worker, enqueue, pick, record,
};
use kohaku::mail::{Failure, FailureClass, Mailer, Outgoing, SendFuture};
use kohaku::time::now_unix;
use rusqlite::Transaction;
use support::*;

static NORMAL: MailKind = MailKind {
    name: "test_normal",
    priority: Priority::Normal,
    token: false,
    lifetime: 48 * 3600,
    give_up: None,
};

static SECURITY: MailKind = MailKind {
    name: "test_security",
    priority: Priority::Security,
    token: false,
    lifetime: 48 * 3600,
    give_up: None,
};

fn invalidate(tx: &Transaction<'_>, id: i64) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE test_tokens SET valid = 0 WHERE outbox_id = ?1",
        [id],
    )
    .map(drop)
}

static TOKEN: MailKind = MailKind {
    name: "test_token",
    priority: Priority::Normal,
    token: true,
    lifetime: 10 * 60,
    give_up: Some(invalidate),
};

static KINDS: &[&MailKind] = &[&NORMAL, &SECURITY, &TOKEN];

/// Records every delivery; subjects starting with `panic`, `reject` or `silent` make
/// it panic, fail permanently or never answer.
#[derive(Default)]
struct FakeMailer {
    sent: Mutex<Vec<String>>,
    active: AtomicUsize,
    peak: AtomicUsize,
}

impl Mailer for FakeMailer {
    fn send(&self, outgoing: Outgoing) -> SendFuture<'_> {
        Box::pin(async move {
            let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            let subject = outgoing
                .message
                .headers()
                .get_raw("Subject")
                .unwrap_or_default()
                .to_owned();
            self.sent.lock().unwrap().push(subject.clone());
            let result = if subject.starts_with("panic") {
                self.active.fetch_sub(1, Ordering::SeqCst);
                panic!("mailer panicked");
            } else if subject.starts_with("reject") {
                Err(Failure {
                    class: FailureClass::Rejected,
                    code: Some(550),
                })
            } else if subject.starts_with("silent") {
                std::future::pending::<()>().await;
                Ok(())
            } else {
                Ok(())
            };
            self.active.fetch_sub(1, Ordering::SeqCst);
            result
        })
    }
}

async fn setup() -> Harness {
    let harness = Harness::new();
    harness
        .app
        .db
        .write(|tx| {
            tx.execute_batch(
                "CREATE TABLE test_tokens (outbox_id INTEGER PRIMARY KEY, valid INTEGER NOT NULL)",
            )
        })
        .await
        .unwrap();
    harness
}

async fn queue(db: &Db, wakeup: &Wakeup, kind: &'static MailKind, subject: &'static str) -> i64 {
    let wakeup = wakeup.clone();
    db.write(move |tx| {
        let id = enqueue(
            tx,
            &wakeup,
            NewMail {
                kind,
                recipient: Recipient::Address("reporter@example.com".to_owned()),
                subject,
                body: "body",
                placeholder: false,
            },
            now_unix(),
        )
        .map_err(|e| match e {
            kohaku::mail::outbox::EnqueueError::Database(e) => e,
            other => panic!("{other:?}"),
        })?;
        if kind.token {
            tx.execute("INSERT INTO test_tokens VALUES (?1, 1)", [id])?;
        }
        Ok::<_, rusqlite::Error>(id)
    })
    .await
    .unwrap()
}

async fn sql<T: Send + 'static>(
    db: &Db,
    query: &'static str,
    map: fn(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> Vec<T> {
    db.read(move |conn| {
        conn.prepare(query)?
            .query_map([], map)?
            .collect::<rusqlite::Result<Vec<T>>>()
    })
    .await
    .unwrap()
}

fn start(
    harness: &Harness,
    mailer: Arc<FakeMailer>,
    timeout: Duration,
) -> tokio::sync::watch::Sender<()> {
    let (stop, stopped) = tokio::sync::watch::channel(());
    let worker = Worker {
        db: Arc::clone(&harness.app.db),
        mailer,
        kinds: KINDS,
        wakeup: harness.app.outbox.clone(),
        sender: serve_config(&[]).sender,
        attempt_timeout: timeout,
    };
    tokio::spawn(worker.run(stopped));
    stop
}

async fn wait_for(what: &str, mut done: impl AsyncFnMut() -> bool) {
    for _ in 0..500 {
        if done().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test]
async fn database_refuses_both_or_neither_recipient_and_never_reuses_ids() {
    let harness = setup().await;
    let insert = |recipient: &'static str| {
        let db = Arc::clone(&harness.app.db);
        async move {
            db.write(move |tx| {
                tx.execute(
                    &format!(
                        "INSERT INTO outbox (kind, {recipient}, subject, body, priority, token, placeholder,
                             next_attempt_at, queued_at, expires_at)
                         VALUES ('k', {values}, 's', 'b', 'normal', 0, 0, 1, 1, 2)",
                        values = match recipient {
                            "user_id, address" => "1, 'a@example.com'",
                            "user_id" => "1",
                            "address" => "'a@example.com'",
                            _ => unreachable!(),
                        }
                    ),
                    [],
                )
                .map(|_| tx.last_insert_rowid())
            })
            .await
        }
    };
    assert!(insert("user_id, address").await.is_err());
    let none = harness
        .app
        .db
        .write(|tx| {
            tx.execute(
                "INSERT INTO outbox (kind, subject, body, priority, token, placeholder,
                     next_attempt_at, queued_at, expires_at)
                 VALUES ('k', 's', 'b', 'normal', 0, 0, 1, 1, 2)",
                [],
            )
        })
        .await;
    assert!(none.is_err());
    let first = insert("address").await.unwrap();
    harness
        .app
        .db
        .write(move |tx| tx.execute("DELETE FROM outbox WHERE id = ?1", [first]))
        .await
        .unwrap();
    let second = insert("address").await.unwrap();
    assert!(second > first);
}

#[tokio::test]
async fn enqueue_rules() {
    let harness = setup().await;
    let wakeup = harness.app.outbox.clone();
    // Rolled back: no row.
    let rolled_back: Result<(), rusqlite::Error> = harness
        .app
        .db
        .write(move |tx| {
            enqueue(
                tx,
                &wakeup,
                NewMail {
                    kind: &NORMAL,
                    recipient: Recipient::Address("a@example.com".to_owned()),
                    subject: "s",
                    body: "b",
                    placeholder: false,
                },
                now_unix(),
            )
            .unwrap();
            Err(rusqlite::Error::InvalidQuery)
        })
        .await;
    assert!(rolled_back.is_err());
    let wakeup = harness.app.outbox.clone();
    let bad = harness
        .app
        .db
        .write(move |tx| {
            Ok::<_, rusqlite::Error>(
                enqueue(
                    tx,
                    &wakeup,
                    NewMail {
                        kind: &NORMAL,
                        recipient: Recipient::Address(
                            "victim@example.com\r\nBcc: list@example.net".to_owned(),
                        ),
                        subject: "s",
                        body: "b",
                        placeholder: false,
                    },
                    now_unix(),
                )
                .is_err(),
            )
        })
        .await
        .unwrap();
    assert!(bad);
    assert!(
        sql(&harness.app.db, "SELECT id FROM outbox", |r| r
            .get::<_, i64>(0))
        .await
        .is_empty()
    );
    let now = now_unix();
    let id = queue(&harness.app.db, &harness.app.outbox, &NORMAL, "hello").await;
    let rows = sql(
        &harness.app.db,
        "SELECT id, next_attempt_at, queued_at FROM outbox",
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        },
    )
    .await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, id);
    assert_eq!(rows[0].1, rows[0].2, "due at once");
    assert!(rows[0].1 >= now);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn public_mail_flood_does_not_delay_security_mail() {
    let harness = setup().await;
    let db = &harness.app.db;
    let wakeup = &harness.app.outbox;
    for n in 0..50 {
        let subject: &'static str = match n {
            7 => "panic row",
            8 => "reject row",
            _ => "normal row",
        };
        queue(db, wakeup, &NORMAL, subject).await;
    }
    let waiting = queue(db, wakeup, &NORMAL, "waiting row").await;
    db.write(move |tx| {
        tx.execute("UPDATE outbox SET next_attempt_at = next_attempt_at + 7200, attempts = 4 WHERE id = ?1", [waiting])
    })
    .await
    .unwrap();
    queue(db, wakeup, &SECURITY, "security row").await;
    let mailer = Arc::new(FakeMailer::default());
    let _stop = start(&harness, Arc::clone(&mailer), Duration::from_secs(60));
    wait_for("51 attempts", async || {
        mailer.sent.lock().unwrap().len() >= 51
    })
    .await;
    // A row queued later is still delivered without waiting for the 2-hour row.
    queue(db, wakeup, &NORMAL, "late row").await;
    wait_for("the late row", async || {
        mailer.sent.lock().unwrap().len() >= 52
    })
    .await;
    let sent = mailer.sent.lock().unwrap().clone();
    assert_eq!(sent[0], "security row");
    assert!(!sent.contains(&"waiting row".to_owned()));
    assert_eq!(
        mailer.peak.load(Ordering::SeqCst),
        1,
        "one attempt at a time"
    );
    let failing = sql(
        db,
        "SELECT subject, attempts, outcome FROM outbox WHERE outcome IS NULL ORDER BY id",
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        },
    )
    .await;
    assert_eq!(
        failing,
        [
            ("panic row".to_owned(), 1, None),
            ("reject row".to_owned(), 1, None),
            ("waiting row".to_owned(), 4, None),
        ]
    );
    let sent_rows = sql(
        db,
        "SELECT count(*) FROM outbox WHERE outcome = 'sent' AND attempts = 1",
        |r| r.get::<_, i64>(0),
    )
    .await;
    assert_eq!(sent_rows, [50]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn acceptance_defines_sent() {
    let harness = setup().await;
    let db = &harness.app.db;
    let silent = queue(db, &harness.app.outbox, &NORMAL, "silent row").await;
    let mailer = Arc::new(FakeMailer::default());
    let stop = start(&harness, Arc::clone(&mailer), Duration::from_millis(300));
    wait_for("the abandoned attempt", async || {
        let rows = sql(db, "SELECT attempts FROM outbox", |r| r.get::<_, u32>(0)).await;
        rows == [1]
    })
    .await;
    let state = sql(
        db,
        "SELECT outcome, next_attempt_at - queued_at FROM outbox",
        |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, i64>(1)?)),
    )
    .await;
    assert_eq!(state[0].0, None, "not sent");
    assert!((60..=62).contains(&state[0].1), "retried after 1 min");
    // Shutdown during an attempt records nothing.
    db.write(move |tx| {
        tx.execute(
            "UPDATE outbox SET next_attempt_at = 0 WHERE id = ?1",
            [silent],
        )
    })
    .await
    .unwrap();
    harness.app.outbox.wake();
    wait_for("the second attempt", async || {
        mailer.sent.lock().unwrap().len() >= 2
    })
    .await;
    stop.send(()).unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let rows = sql(db, "SELECT attempts, outcome FROM outbox", |r| {
        Ok((r.get::<_, u32>(0)?, r.get::<_, Option<String>>(1)?))
    })
    .await;
    assert_eq!(rows, [(1, None)]);
}

#[tokio::test]
async fn token_rows_are_deleted_when_sent_or_given_up() {
    let harness = setup().await;
    let db = &harness.app.db;
    let delivered = queue(db, &harness.app.outbox, &TOKEN, "one-time 482913").await;
    let failing = queue(db, &harness.app.outbox, &TOKEN, "one-time 482913").await;
    let now = now_unix();
    db.write(move |tx| {
        record(tx, KINDS, delivered, Outcome::Sent, now)?;
        let failure = Outcome::Failed(Failure {
            class: FailureClass::Connection,
            code: None,
        });
        // Third failure at 6 min: the next attempt (36 min) is past the 10-minute expiry.
        for ended in [now, now + 60, now + 360] {
            record(tx, KINDS, failing, failure, ended)?;
        }
        Ok::<_, rusqlite::Error>(())
    })
    .await
    .unwrap();
    assert!(
        sql(db, "SELECT id FROM outbox", |r| r.get::<_, i64>(0))
            .await
            .is_empty()
    );
    let tokens = sql(
        db,
        "SELECT outbox_id, valid FROM test_tokens ORDER BY outbox_id",
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
    )
    .await;
    assert_eq!(
        tokens,
        [(delivered, 1), (failing, 0)],
        "the given-up code is no longer accepted"
    );
    // Nothing of the token mail survives into a backup.
    let target = TempDir::new();
    let path = target.path().join("b.db");
    kohaku::db::backup::backup_to_file(&harness.data, &secret(), &path).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(!bytes.windows(6).any(|w| w == b"482913"));
}

#[tokio::test]
async fn placeholder_row_is_never_sent() {
    let harness = setup().await;
    let wakeup = harness.app.outbox.clone();
    harness
        .app
        .db
        .write(move |tx| {
            enqueue(
                tx,
                &wakeup,
                NewMail {
                    kind: &NORMAL,
                    recipient: Recipient::Address("nobody@example.com".to_owned()),
                    subject: "placeholder",
                    body: "b",
                    placeholder: true,
                },
                now_unix(),
            )
            .map_err(|_| rusqlite::Error::InvalidQuery)
        })
        .await
        .unwrap();
    let picked = harness
        .app
        .db
        .write(|tx| pick(tx, KINDS, now_unix()))
        .await
        .unwrap();
    assert_eq!(picked, Pick::Idle(None));
    assert!(
        sql(&harness.app.db, "SELECT id FROM outbox", |r| r
            .get::<_, i64>(0))
        .await
        .is_empty()
    );
}

#[tokio::test]
async fn restart_keeps_the_schedule() {
    let harness = setup().await;
    let db = &harness.app.db;
    let a = queue(db, &harness.app.outbox, &NORMAL, "row a").await;
    let b = queue(db, &harness.app.outbox, &TOKEN, "row b").await;
    let queued = now_unix();
    let third_ended = queued + 1000;
    db.write(move |tx| {
        let failure = Outcome::Failed(Failure {
            class: FailureClass::Temporary,
            code: Some(451),
        });
        for ended in [queued + 1, queued + 100, third_ended] {
            record(tx, KINDS, a, failure, ended)?;
        }
        Ok::<_, rusqlite::Error>(())
    })
    .await
    .unwrap();
    // Restart 10 minutes after A's third failure and 15 minutes after B was queued.
    let restart = (third_ended + 600).max(queued + 900);
    let picked = db.write(move |tx| pick(tx, KINDS, restart)).await.unwrap();
    assert_eq!(
        picked,
        Pick::Idle(Some(third_ended + 30 * 60)),
        "A's fourth attempt is 30 min after its third"
    );
    let rows = sql(db, "SELECT id FROM outbox", |r| r.get::<_, i64>(0)).await;
    assert_eq!(rows, [a], "B was given up without an attempt");
    let tokens = sql(
        db,
        "SELECT valid FROM test_tokens WHERE outbox_id = 2",
        |r| r.get::<_, i64>(0),
    )
    .await;
    assert_eq!(tokens, [0]);
    let _ = b;
    let due = db
        .write(move |tx| pick(tx, KINDS, third_ended + 30 * 60))
        .await
        .unwrap();
    assert!(matches!(due, Pick::Due(row) if row.id == a));
}

#[tokio::test]
async fn deleted_row_stays_deleted() {
    let harness = setup().await;
    let db = &harness.app.db;
    let id = queue(db, &harness.app.outbox, &NORMAL, "row").await;
    let picked = db.write(|tx| pick(tx, KINDS, now_unix())).await.unwrap();
    assert!(matches!(picked, Pick::Due(ref row) if row.id == id));
    // Deleted during its attempt; both outcomes leave it deleted.
    db.write(move |tx| tx.execute("DELETE FROM outbox WHERE id = ?1", [id]))
        .await
        .unwrap();
    let now = now_unix();
    db.write(move |tx| {
        record(tx, KINDS, id, Outcome::Sent, now)?;
        record(
            tx,
            KINDS,
            id,
            Outcome::Failed(Failure {
                class: FailureClass::Timeout,
                code: None,
            }),
            now,
        )
    })
    .await
    .unwrap();
    assert!(
        sql(db, "SELECT id FROM outbox", |r| r.get::<_, i64>(0))
            .await
            .is_empty()
    );
    assert_eq!(
        db.write(|tx| pick(tx, KINDS, now_unix())).await.unwrap(),
        Pick::Idle(None)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn account_rows_follow_the_account() {
    let harness = setup().await;
    let db = &harness.app.db;
    let kept = harness
        .account("kept@example.org", "maintainer", false)
        .await;
    let deleted = harness
        .account("gone@example.org", "maintainer", false)
        .await;
    let wakeup = harness.app.outbox.clone();
    let now = now_unix();
    let (kept_id, deleted_id) = (kept.id, deleted.id);
    db.write(move |tx| {
        for (id, subject) in [
            (deleted_id, "to the deleted account"),
            (kept_id, "to the account"),
        ] {
            enqueue(
                tx,
                &wakeup,
                NewMail {
                    kind: &NORMAL,
                    recipient: Recipient::User(id),
                    subject,
                    body: "b",
                    placeholder: false,
                },
                now,
            )
            .unwrap();
        }
        tx.execute("DELETE FROM users WHERE id = ?1", [deleted_id])?;
        // The address is resolved at delivery time.
        tx.execute(
            "UPDATE users SET email = 'moved@example.org' WHERE id = ?1",
            [kept_id],
        )
        .map(drop)
    })
    .await
    .unwrap();
    let refused = db
        .write(move |tx| {
            enqueue(
                tx,
                &Wakeup::default(),
                NewMail {
                    kind: &NORMAL,
                    recipient: Recipient::User(999),
                    subject: "s",
                    body: "b",
                    placeholder: false,
                },
                now,
            )
            .map_err(|_| rusqlite::Error::InvalidQuery)
        })
        .await;
    assert!(refused.is_err(), "the database refuses a missing account");
    let picked = db.write(move |tx| pick(tx, KINDS, now)).await.unwrap();
    match picked {
        Pick::Due(row) => {
            assert_eq!(row.subject, "to the account");
            assert_eq!(row.address, "moved@example.org");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(harness.count("SELECT count(*) FROM outbox").await, 1);
    let mailer = Arc::new(FakeMailer::default());
    let stop = start(&harness, Arc::clone(&mailer), Duration::from_secs(5));
    wait_for("the delivery", async || {
        harness
            .count("SELECT count(*) FROM outbox WHERE outcome = 'sent'")
            .await
            == 1
    })
    .await;
    stop.send(()).unwrap();
    assert_eq!(*mailer.sent.lock().unwrap(), ["to the account"]);
}
