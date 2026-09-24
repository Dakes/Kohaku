//! The audit log's rules, enforced by the database and the typed helper (audit-log;
//! change foundation D8, D19).

mod support;

use axum::body::Body;
use kohaku::audit::{Action, Actor, Target, audit};
use kohaku::db::migrate::{MIGRATIONS, open_and_prepare};
use kohaku::routing::table::{HostKind, table};
use rusqlite::{Connection, params};
use support::*;

const DAY: i64 = 86_400;

fn database() -> (TempDir, Connection) {
    let (dir, data) = data_dir();
    let conn = open_and_prepare(&data, &secret(), MIGRATIONS, 1_800_000_000).unwrap();
    (dir, conn)
}

fn now() -> i64 {
    kohaku::time::now_unix()
}

type Entry = (String, Option<i64>, String, String, Option<i64>, i64);

fn insert(
    conn: &Connection,
    entry: (&str, Option<i64>, &str, &str, Option<i64>, i64),
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO audit_log (actor_label, actor_id, action, target_type, target_id, time)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![entry.0, entry.1, entry.2, entry.3, entry.4, entry.5],
    )
}

fn entries(conn: &Connection) -> Vec<Entry> {
    conn.prepare("SELECT actor_label, actor_id, action, target_type, target_id, time FROM audit_log ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn count(conn: &Connection) -> i64 {
    conn.query_row("SELECT count(*) FROM audit_log", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn entry_read_back() {
    let (_dir, conn) = database();
    let time = now();
    insert(
        &conn,
        (
            "maintainer",
            Some(7),
            "sample.update",
            "sample",
            Some(42),
            time,
        ),
    )
    .unwrap();
    assert_eq!(
        entries(&conn),
        [(
            "maintainer".to_owned(),
            Some(7),
            "sample.update".to_owned(),
            "sample".to_owned(),
            Some(42),
            time
        )]
    );
    let columns: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('audit_log')")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        columns,
        [
            "id",
            "actor_label",
            "actor_id",
            "action",
            "target_type",
            "target_id",
            "time"
        ]
    );
}

#[test]
fn malformed_inconsistent_or_attacker_values_are_refused() {
    let (_dir, conn) = database();
    let t = now();
    for identifier in [
        "",
        "a@b",
        "a b",
        "Sample",
        "sample.Update",
        "1sample",
        "_sample",
        "sample._x",
        "sample.1x",
        "sample.",
        ".sample",
        "sample..x",
        "victim@example.com",
        "203.0.113.7",
        "Crash on start",
    ] {
        assert!(
            insert(&conn, ("cli", None, identifier, "instance", None, t)).is_err(),
            "action {identifier:?}"
        );
        assert!(
            insert(
                &conn,
                ("cli", None, "sample.update", identifier, Some(1), t)
            )
            .is_err(),
            "target {identifier:?}"
        );
    }
    assert!(insert(&conn, ("root", None, "a", "instance", None, t)).is_err());
    assert!(insert(&conn, ("cli", Some(1), "a", "instance", None, t)).is_err());
    assert!(insert(&conn, ("admin", None, "a", "instance", None, t)).is_err());
    assert!(insert(&conn, ("cli", None, "a", "instance", Some(1), t)).is_err());
    assert!(insert(&conn, ("cli", None, "a", "sample", None, t)).is_err());
    // STRICT stores a lossless integer string as the integer; anything else is refused.
    for text_id in ["7.5", "victim@example.com", "203.0.113.7", ""] {
        let refused = conn.execute(
            "INSERT INTO audit_log (actor_label, actor_id, action, target_type, target_id, time)
             VALUES ('admin', ?1, 'a', 'sample', 1, ?2)",
            params![text_id, t],
        );
        assert!(refused.is_err(), "{text_id}");
    }
    let real = conn.execute(
        "INSERT INTO audit_log (actor_label, actor_id, action, target_type, target_id, time)
         VALUES ('admin', 1, 'a', 'sample', 1.5, ?1)",
        [t],
    );
    assert!(real.is_err());
    assert_eq!(count(&conn), 0);
}

#[test]
fn recent_entries_cannot_be_rewritten_or_erased() {
    let (_dir, conn) = database();
    insert(
        &conn,
        (
            "maintainer",
            Some(7),
            "sample.update",
            "sample",
            Some(42),
            now() - 364 * DAY,
        ),
    )
    .unwrap();
    insert(
        &conn,
        (
            "maintainer",
            Some(7),
            "sample.update",
            "sample",
            Some(42),
            now() - 366 * DAY,
        ),
    )
    .unwrap();
    for change in [
        "UPDATE audit_log SET actor_id = 1",
        "UPDATE audit_log SET actor_label = 'cli', actor_id = NULL",
        "UPDATE audit_log SET time = 0",
    ] {
        assert!(conn.execute(change, []).is_err(), "{change}");
    }
    assert!(
        conn.execute("DELETE FROM audit_log WHERE time > ?1", [now() - 365 * DAY])
            .is_err()
    );
    assert_eq!(
        conn.execute("DELETE FROM audit_log WHERE time < ?1", [now() - 365 * DAY])
            .unwrap(),
        1
    );
    assert_eq!(count(&conn), 1);
}

#[test]
fn entry_and_change_are_atomic() {
    let (_dir, mut conn) = database();
    conn.execute_batch(
        "CREATE TABLE sample (id INTEGER PRIMARY KEY AUTOINCREMENT, v INTEGER) STRICT",
    )
    .unwrap();
    // Committed: change and entry.
    let tx = conn.transaction().unwrap();
    tx.execute("INSERT INTO sample (v) VALUES (1)", []).unwrap();
    audit(
        &tx,
        Actor::Cli,
        Action::InstanceRestore,
        Target::Instance,
        now(),
    )
    .unwrap();
    tx.commit().unwrap();
    // Fails after the entry, before commit: neither.
    let tx = conn.transaction().unwrap();
    tx.execute("INSERT INTO sample (v) VALUES (2)", []).unwrap();
    audit(
        &tx,
        Actor::Cli,
        Action::InstanceRestore,
        Target::Instance,
        now(),
    )
    .unwrap();
    drop(tx);
    // The database refuses the entry: the action fails and its change rolls back.
    let tx = conn.transaction().unwrap();
    tx.execute("INSERT INTO sample (v) VALUES (3)", []).unwrap();
    let refused = tx.execute(
        "INSERT INTO audit_log (actor_label, actor_id, action, target_type, target_id, time)
         VALUES ('cli', NULL, 'Bad Action', 'instance', NULL, 1)",
        [],
    );
    assert!(refused.is_err());
    drop(tx);
    let values: Vec<i64> = conn
        .prepare("SELECT v FROM sample")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(values, [1]);
    assert_eq!(count(&conn), 1);
    let (label, actor, action, target, target_id, _) = entries(&conn).remove(0);
    assert_eq!(
        (
            label.as_str(),
            actor,
            action.as_str(),
            target.as_str(),
            target_id
        ),
        ("cli", None, "instance.restore", "instance", None)
    );
}

/// The table holding the records an audit target names; `None` for the instance.
fn target_table(target: Target) -> Option<&'static str> {
    match target {
        Target::Instance => None,
        Target::User(_) => Some("users"),
        Target::Project(_) => Some("projects"),
    }
}

/// The table holding the users an actor names; `None` for the CLI.
fn actor_table(actor: Actor) -> Option<&'static str> {
    match actor {
        Actor::Cli => None,
        Actor::User { .. } => Some("users"),
    }
}

#[test]
fn audited_kinds_live_in_autoincrement_tables() {
    let (_dir, conn) = database();
    let user = Actor::User {
        id: 1,
        role: kohaku::auth::Role::Admin,
    };
    let tables: Vec<&str> = [
        target_table(Target::Instance),
        target_table(Target::User(1)),
        target_table(Target::Project(1)),
        actor_table(Actor::Cli),
        actor_table(user),
    ]
    .into_iter()
    .flatten()
    .collect();
    for table in tables {
        let sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE name = ?1",
                [table],
                |r| r.get(0),
            )
            .unwrap();
        assert!(sql.contains("AUTOINCREMENT"), "{table}");
    }
    for action in Action::ALL {
        let id = action.identifier();
        assert!(
            id.split('.')
                .all(|seg| seg.starts_with(|c: char| c.is_ascii_lowercase()))
        );
    }
}

#[test]
fn entries_outlive_their_records_and_ids_are_never_reused() {
    let (_dir, conn) = database();
    conn.execute_batch(
        "CREATE TABLE sample (id INTEGER PRIMARY KEY AUTOINCREMENT, v INTEGER) STRICT;
         INSERT INTO sample (id, v) VALUES (7, 0), (42, 0);",
    )
    .unwrap();
    let t = now();
    insert(
        &conn,
        ("admin", Some(999), "sample.update", "sample", Some(999), t),
    )
    .unwrap();
    insert(
        &conn,
        (
            "maintainer",
            Some(7),
            "sample.update",
            "sample",
            Some(42),
            t,
        ),
    )
    .unwrap();
    let before = entries(&conn);
    conn.execute_batch("DELETE FROM sample").unwrap();
    assert_eq!(entries(&conn), before);
    conn.execute("INSERT INTO sample (v) VALUES (1)", [])
        .unwrap();
    let new_id: i64 = conn
        .query_row("SELECT max(id) FROM sample", [], |r| r.get(0))
        .unwrap();
    assert!(new_id > 42);
    assert!(!before.iter().any(|e| e.4 == Some(new_id)));
}

#[tokio::test]
async fn anonymous_request_flood_records_nothing() {
    let mut routes = table();
    routes.push(synthetic_route(
        HostKind::Main,
        "/items/{id}",
        axum::routing::get(|| async { "x" }),
    ));
    let harness = Harness::with(routes, &[]);
    let audit_count = || {
        let db = std::sync::Arc::clone(&harness.app.db);
        async move {
            db.read(|c| c.query_row("SELECT count(*) FROM audit_log", [], |r| r.get::<_, i64>(0)))
                .await
                .unwrap()
        }
    };
    let before = audit_count().await;
    for host in [MAIN_HOST, "evil.example", ""] {
        for path in [
            "/",
            "/admin",
            "/admin/x",
            "/admin/projects",
            "/admin/p/demo/settings",
            "/admin/p/demo/delete",
            "/p/demo",
            "/items/1",
            "/nothing",
            "/healthz",
            "/.well-known/kohaku/tls-ask?domain=a.test",
        ] {
            for method in ["GET", "POST", "PUT", "DELETE"] {
                harness
                    .send(
                        request(method, host, path)
                            .header("origin", "https://kohaku.example.org")
                            .header("x-forwarded-for", "203.0.113.7")
                            .body(Body::from("victim@example.com"))
                            .unwrap(),
                    )
                    .await;
            }
        }
    }
    assert_eq!(audit_count().await, before);
}
