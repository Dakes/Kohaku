//! Migration runner, downgrade guard, keycheck and pre-migration copy (data-storage,
//! configuration: Database keycheck; change foundation D6, D8).

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use kohaku::db::migrate::{MIGRATIONS, MigrateError, Migration, open_and_prepare, user_version};
use kohaku::db::{DataDir, Role, open};
use rusqlite::Connection;
use rusqlite::types::Value;
use support::*;

const NOW: i64 = 1_800_000_000;

fn synthetic(version: u32, name: &'static str, sql: &'static str) -> Migration {
    Migration { version, name, sql }
}

/// Migration 1 plus three synthetic ones, the third rebuilding a parent table.
fn list(n: usize) -> Vec<Migration> {
    let all = [
        MIGRATIONS[0],
        synthetic(
            2,
            "0002_two",
            "CREATE TABLE two (id INTEGER PRIMARY KEY) STRICT;",
        ),
        synthetic(
            3,
            "0003_three",
            "CREATE TABLE three (id INTEGER PRIMARY KEY,
                 two_id INTEGER NOT NULL REFERENCES two (id) ON DELETE CASCADE) STRICT;",
        ),
        synthetic(
            4,
            "0004_four",
            "CREATE TABLE two_new (id INTEGER PRIMARY KEY, extra TEXT) STRICT;
             INSERT INTO two_new (id) SELECT id FROM two;
             DROP TABLE two;
             ALTER TABLE two_new RENAME TO two;",
        ),
    ];
    all[..n].to_vec()
}

fn prepare(data: &DataDir, migrations: &[Migration]) -> Result<Connection, MigrateError> {
    open_and_prepare(data, &secret(), migrations, NOW)
}

fn tables(conn: &Connection) -> BTreeSet<String> {
    let mut statement = conn
        .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn version_of(data: &DataDir) -> u32 {
    user_version(&open(&data.database(), Role::Command).unwrap()).unwrap()
}

#[test]
fn embedded_migrations_match_the_directory() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut files: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    files.sort();
    let names: Vec<String> = MIGRATIONS
        .iter()
        .map(|m| format!("{}.sql", m.name))
        .collect();
    assert_eq!(files, names);
    for (i, migration) in MIGRATIONS.iter().enumerate() {
        assert_eq!(migration.version as usize, i + 1);
        assert!(
            migration
                .name
                .starts_with(&format!("{:04}_", migration.version))
        );
        let text = fs::read_to_string(dir.join(format!("{}.sql", migration.name))).unwrap();
        assert_eq!(text, migration.sql);
        // The runner owns the transaction; trigger bodies' BEGIN ... END are fine.
        let upper = migration.sql.to_uppercase();
        for statement in [
            "BEGIN;",
            "BEGIN TRANSACTION",
            "BEGIN DEFERRED",
            "BEGIN IMMEDIATE",
            "BEGIN EXCLUSIVE",
            "COMMIT",
            "END TRANSACTION",
            "ROLLBACK",
            "SAVEPOINT",
            "PRAGMA",
            "VACUUM",
        ] {
            assert!(
                !upper.contains(statement),
                "{} contains {statement}",
                migration.name
            );
        }
    }
}

#[test]
fn new_older_and_current_databases() {
    let (_dir, data) = data_dir();
    let conn = prepare(&data, &list(3)).unwrap();
    assert_eq!(user_version(&conn).unwrap(), 3);
    assert!(tables(&conn).is_superset(&["meta", "two", "three"].map(String::from).into()));
    drop(conn);
    assert!(!data.backups().exists(), "no copy for a new database");

    let (_dir, data) = data_dir();
    prepare(&data, &list(1)).unwrap();
    let conn = prepare(&data, &list(3)).unwrap();
    assert_eq!(user_version(&conn).unwrap(), 3);
    drop(conn);

    let backups_before = fs::read_dir(data.backups()).unwrap().count();
    prepare(&data, &list(3)).unwrap();
    assert_eq!(version_of(&data), 3);
    assert_eq!(
        fs::read_dir(data.backups()).unwrap().count(),
        backups_before,
        "no copy for a current database"
    );
}

#[test]
fn dangling_foreign_key_rolls_back_only_that_migration() {
    let (_dir, data) = data_dir();
    let mut migrations = list(2);
    migrations.push(synthetic(
        3,
        "0003_dangling",
        "CREATE TABLE p (id INTEGER PRIMARY KEY) STRICT;
         CREATE TABLE c (id INTEGER PRIMARY KEY, p INTEGER REFERENCES p (id)) STRICT;
         INSERT INTO c VALUES (1, 42);",
    ));
    let error = prepare(&data, &migrations).err().unwrap();
    assert!(matches!(
        error,
        MigrateError::ForeignKeyViolation {
            migration: "0003_dangling"
        }
    ));
    assert!(error.to_string().contains("0003_dangling"));
    let conn = open(&data.database(), Role::Command).unwrap();
    assert_eq!(
        user_version(&conn).unwrap(),
        2,
        "migration 2 of that start stays"
    );
    let names = tables(&conn);
    assert!(names.contains("two") && !names.contains("p") && !names.contains("c"));
}

#[test]
fn failing_statement_in_k_leaves_k_minus_one() {
    let (_dir, data) = data_dir();
    prepare(&data, &list(1)).unwrap();
    let mut migrations = list(2);
    migrations.push(synthetic(
        3,
        "0003_broken",
        "CREATE TABLE half (id INTEGER PRIMARY KEY) STRICT; SELECT nonsense FROM nowhere;",
    ));
    let error = prepare(&data, &migrations).err().unwrap();
    assert!(matches!(
        error,
        MigrateError::Statement {
            migration: "0003_broken",
            ..
        }
    ));
    let conn = open(&data.database(), Role::Command).unwrap();
    assert_eq!(user_version(&conn).unwrap(), 2);
    assert!(!tables(&conn).contains("half"));
}

/// Child helper: starts migration 2 on the database and waits to be killed mid-way.
#[test]
#[ignore = "helper process for interrupted_migration_is_applied_again"]
fn child_interrupted_migration() {
    let Some(root) = child_argument() else { return };
    let conn = open(&DataDir::new(root).database(), Role::Writer).unwrap();
    conn.pragma_update(None, "foreign_keys", false).unwrap();
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    conn.execute_batch(list(2)[1].sql).unwrap();
    conn.pragma_update(None, "user_version", 2).unwrap();
    child_ready_and_wait();
}

#[test]
fn interrupted_migration_is_applied_again() {
    let (dir, data) = data_dir();
    prepare(&data, &list(1)).unwrap();
    let mut child = spawn_child("child_interrupted_migration", dir.path().to_str().unwrap());
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(version_of(&data), 1);
    assert!(!tables(&open(&data.database(), Role::Command).unwrap()).contains("two"));
    let conn = prepare(&data, &list(2)).unwrap();
    assert_eq!(user_version(&conn).unwrap(), 2);
    assert!(tables(&conn).contains("two"));
}

#[test]
fn older_release_started_on_a_newer_database() {
    let (_dir, data) = data_dir();
    prepare(&data, &list(4)).unwrap();
    let error = prepare(&data, &list(3)).err().unwrap();
    assert!(matches!(
        error,
        MigrateError::Downgrade {
            database: 4,
            binary: 3
        }
    ));
    let text = error.to_string();
    assert!(text.contains('4') && text.contains('3') && text.contains("older"));
    assert_eq!(version_of(&data), 4);
    assert!(!data.backups().exists());
}

#[test]
fn version_zero_with_a_schema_is_not_kohaku() {
    let (_dir, data) = data_dir();
    Connection::open(data.database())
        .unwrap()
        .execute_batch("CREATE TABLE other (x)")
        .unwrap();
    let error = prepare(&data, &list(1)).err().unwrap();
    assert!(matches!(error, MigrateError::NotKohaku));
    let conn = open(&data.database(), Role::Command).unwrap();
    assert_eq!(user_version(&conn).unwrap(), 0);
    assert_eq!(tables(&conn), BTreeSet::from(["other".to_owned()]));
}

/// One sample row per table, parents first (design §12). `None`: the row migration 1
/// writes.
type Samples = &'static [(&'static str, Option<&'static str>)];

const SYNTHETIC_SAMPLES: Samples = &[
    ("meta", None),
    ("outbox", SHIPPED_SAMPLES[1].1),
    ("audit_log", SHIPPED_SAMPLES[2].1),
    ("two", Some("INSERT INTO two (id) VALUES (7)")),
    (
        "three",
        Some("INSERT INTO three (id, two_id) VALUES (1, 7)"),
    ),
];

/// Sample rows of the shipped schema; each change adds its tables'.
const SHIPPED_SAMPLES: Samples = &[
    ("meta", None),
    (
        "outbox",
        Some(
            "INSERT INTO outbox (kind, address, subject, body, priority, token, placeholder,
                 next_attempt_at, queued_at, expires_at)
             VALUES ('sample', 'a@example.com', 's', 'b', 'normal', 0, 0, 1, 1, 2)",
        ),
    ),
    (
        "audit_log",
        Some(
            "INSERT INTO audit_log (actor_label, actor_id, action, target_type, target_id, time)
             VALUES ('cli', NULL, 'instance.restore', 'instance', NULL, unixepoch())",
        ),
    ),
    (
        "users",
        Some(
            "INSERT INTO users (id, email, password_hash, role, totp_nonce, totp_last_step,
                 failed_logins, locked_until, created_at)
             VALUES (7, 'admin@example.org', '$argon2id$x', 'admin', zeroblob(16), 5, 2, 9, 1)",
        ),
    ),
    (
        "sessions",
        Some(
            "INSERT INTO sessions (token_hash, user_id, csrf_token, created_at, last_seen,
                 pending_totp_nonce)
             VALUES (zeroblob(32), 7, zeroblob(32), 1, 2, zeroblob(16))",
        ),
    ),
    (
        "known_devices",
        Some(
            "INSERT INTO known_devices (user_id, token_hash, created_at)
             VALUES (7, zeroblob(32), 1)",
        ),
    ),
    (
        "recovery_codes",
        Some("INSERT INTO recovery_codes (user_id, code_hash) VALUES (7, zeroblob(32))"),
    ),
    (
        "tokens",
        Some(
            "INSERT INTO tokens (purpose, token_hash, user_id, expires_at)
             VALUES ('reset', zeroblob(32), 7, 3)",
        ),
    ),
];

/// Migrates a new database to each version k, inserts a sample row into every table,
/// applies the rest and checks that no row was deleted or changed.
fn check_sample_rows(full: &[Migration], samples: Samples) {
    for k in 1..=full.len() {
        let (_dir, data) = data_dir();
        let conn = prepare(&data, &full[..k]).unwrap();
        let present = tables(&conn);
        let known: BTreeSet<String> = samples.iter().map(|(t, _)| (*t).to_owned()).collect();
        let missing: Vec<_> = present.difference(&known).collect();
        assert!(
            missing.is_empty(),
            "tables without a sample row: {missing:?}"
        );
        let mut before = BTreeMap::new();
        for (table, sample) in samples.iter().filter(|(t, _)| present.contains(*t)) {
            if let Some(sql) = sample {
                conn.execute(sql, []).unwrap();
            }
            let cols = columns(&conn, table);
            before.insert(*table, (cols.clone(), rows(&conn, table, &cols)));
        }
        drop(conn);
        let conn = prepare(&data, full).unwrap();
        for (table, (cols, old)) in &before {
            assert!(!old.is_empty(), "table {table} has its sample row");
            assert_eq!(
                &rows(&conn, table, cols),
                old,
                "table {table} after k = {k}"
            );
        }
        assert!(
            !conn
                .prepare("PRAGMA foreign_key_check")
                .unwrap()
                .exists([])
                .unwrap()
        );
        drop(conn);
        for role in [Role::Writer, Role::Reader, Role::Command] {
            let conn = open(&data.database(), role).unwrap();
            let on: i64 = conn
                .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
                .unwrap();
            assert_eq!(on, 1);
        }
    }
}

fn rows(conn: &Connection, table: &str, columns: &[String]) -> Vec<Vec<Value>> {
    let sql = format!("SELECT {} FROM {table} ORDER BY rowid", columns.join(", "));
    let mut statement = conn.prepare(&sql).unwrap();
    statement
        .query_map([], |row| {
            (0..columns.len()).map(|i| row.get::<_, Value>(i)).collect()
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn columns(conn: &Connection, table: &str) -> Vec<String> {
    let mut statement = conn
        .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
        .unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn table_rebuilds_keep_child_rows() {
    check_sample_rows(&list(4), SYNTHETIC_SAMPLES);
}

#[test]
fn shipped_migrations_keep_sample_rows() {
    check_sample_rows(MIGRATIONS, SHIPPED_SAMPLES);
}

#[test]
#[should_panic(expected = "tables without a sample row")]
fn a_table_without_a_sample_row_fails() {
    check_sample_rows(&list(3), &SYNTHETIC_SAMPLES[..4]);
}

#[test]
fn wrong_secret_refused() {
    let (_dir, data) = data_dir();
    open_and_prepare(&data, &secret(), &list(1), NOW).unwrap();
    open_and_prepare(&data, &secret(), &list(1), NOW).unwrap();
    let error = open_and_prepare(&data, &other_secret(), &list(2), NOW)
        .err()
        .unwrap();
    assert!(matches!(error, MigrateError::KeycheckMismatch));
    let text = error.to_string();
    assert_eq!(text, "KOHAKU_SECRET does not match the database");
    assert_eq!(version_of(&data), 1);
    assert!(!data.backups().exists(), "no copy after a secret mismatch");
}

#[test]
fn keycheck_less_database_is_completed_by_serve() {
    let (_dir, data) = data_dir();
    drop(prepare(&data, &list(1)).unwrap());
    open(&data.database(), Role::Command)
        .unwrap()
        .execute("DELETE FROM meta", [])
        .unwrap();
    prepare(&data, &list(1)).unwrap();
    let conn = open(&data.database(), Role::Command).unwrap();
    let stored: Vec<u8> = conn
        .query_row("SELECT keycheck FROM meta", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stored, secret().keycheck());
}

#[test]
fn copy_before_upgrading() {
    let (_dir, data) = data_dir();
    let conn = prepare(&data, &list(3)).unwrap();
    conn.execute("INSERT INTO two (id) VALUES (5)", []).unwrap();
    drop(conn);
    let backups = data.ensure_backups().unwrap();
    for name in [
        "pre-migrate-v1-1790000000.db",
        "pre-migrate-v2-1795000000.db",
        "nightly.db",
    ] {
        fs::write(backups.join(name), b"x").unwrap();
    }
    prepare(&data, &list(4)).unwrap();
    let copy = backups.join(format!("pre-migrate-v3-{NOW}.db"));
    assert_eq!(mode(&copy), 0o600);
    let conn = Connection::open(&copy).unwrap();
    assert_eq!(user_version(&conn).unwrap(), 3);
    assert!(!columns(&conn, "two").contains(&"extra".to_owned()));
    let n: i64 = conn
        .query_row("SELECT count(*) FROM two", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
    drop(conn);
    let mut left: Vec<String> = fs::read_dir(&backups)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| !n.ends_with("-wal") && !n.ends_with("-shm"))
        .collect();
    left.sort();
    assert_eq!(
        left,
        [
            "nightly.db".to_owned(),
            "pre-migrate-v2-1795000000.db".to_owned(),
            format!("pre-migrate-v3-{NOW}.db"),
        ]
    );
}

#[test]
fn missing_backups_directory_is_created_private() {
    let (_dir, data) = data_dir();
    prepare(&data, &list(1)).unwrap();
    assert!(!data.backups().exists());
    prepare(&data, &list(2)).unwrap();
    assert_eq!(mode(&data.backups()), 0o700);
    assert!(
        data.backups()
            .join(format!("pre-migrate-v1-{NOW}.db"))
            .is_file()
    );
}

#[test]
fn a_failed_copy_blocks_the_upgrade() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, data) = data_dir();
    prepare(&data, &list(1)).unwrap();
    let backups = data.ensure_backups().unwrap();
    fs::write(backups.join("pre-migrate-v1-1790000000.db"), b"x").unwrap();
    fs::write(backups.join("pre-migrate-v1-1795000000.db"), b"x").unwrap();
    fs::set_permissions(&backups, fs::Permissions::from_mode(0o500)).unwrap();
    let result = prepare(&data, &list(2));
    fs::set_permissions(&backups, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(
        result.err().unwrap(),
        MigrateError::PreMigrationCopy(_)
    ));
    assert_eq!(version_of(&data), 1);
    let mut names: Vec<String> = fs::read_dir(&backups)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "pre-migrate-v1-1790000000.db",
            "pre-migrate-v1-1795000000.db"
        ]
    );
}

#[test]
fn outbox_rebuild_keeps_deleted_ids_unused() {
    let (_dir, data) = data_dir();
    let conn = prepare(&data, &MIGRATIONS[..1]).unwrap();
    for _ in 0..3 {
        conn.execute(SHIPPED_SAMPLES[1].1.unwrap(), []).unwrap();
    }
    conn.execute("DELETE FROM outbox WHERE id = 3", []).unwrap();
    drop(conn);
    let conn = prepare(&data, MIGRATIONS).unwrap();
    conn.execute(SHIPPED_SAMPLES[1].1.unwrap(), []).unwrap();
    let ids: Vec<i64> = conn
        .prepare("SELECT id FROM outbox ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(ids, [1, 2, 4]);
    // The rebuilt table refuses a row naming both recipients or neither, and an
    // account that does not exist.
    for (user, address) in [("7", "'a@example.com'"), ("NULL", "NULL"), ("99", "NULL")] {
        let sql = format!(
            "INSERT INTO outbox (kind, user_id, address, subject, body, priority, token,
                 placeholder, next_attempt_at, queued_at, expires_at)
             VALUES ('sample', {user}, {address}, 's', 'b', 'normal', 0, 0, 1, 1, 2)"
        );
        assert!(conn.execute(&sql, []).is_err(), "{user} {address}");
    }
}
