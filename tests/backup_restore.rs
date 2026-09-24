//! Instance lock, backup and listing (data-storage: Instance lock, Backup to a file,
//! Backup to standard output; operations: Listing backups; change foundation D9).

mod support;

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use kohaku::db::backup::{BackupError, backup_to_file, backup_to_writer, list_backups};
use kohaku::db::lock::{InstanceLock, LockError};
use kohaku::db::migrate::{MIGRATIONS, MigrateError, Migration, open_and_prepare, user_version};
use kohaku::db::{DataDir, Role, open};
use rusqlite::{Connection, OpenFlags};
use support::*;

const NOW: i64 = 1_800_000_000;

/// A served database with a table of rows written in pairs, one pair per transaction.
fn served(data: &DataDir) -> Connection {
    let conn = open_and_prepare(data, &secret(), MIGRATIONS, NOW).unwrap();
    conn.execute_batch("CREATE TABLE marks (id INTEGER PRIMARY KEY, text TEXT NOT NULL) STRICT")
        .unwrap();
    for i in 0..50 {
        write_pair(&conn, i);
    }
    conn
}

fn write_pair(conn: &Connection, i: i64) {
    conn.execute_batch(&format!(
        "BEGIN IMMEDIATE;
         INSERT INTO marks (text) VALUES ('a{i}');
         INSERT INTO marks (text) VALUES ('b{i}');
         COMMIT;"
    ))
    .unwrap();
}

/// Snapshot of a directory: name → (inode, size, modification time).
fn listing(dir: &Path) -> BTreeMap<String, (u64, u64, i64)> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            let m = fs::symlink_metadata(e.path()).unwrap();
            (
                e.file_name().into_string().unwrap(),
                (
                    m.ino(),
                    m.size(),
                    m.mtime_nsec() + m.mtime() * 1_000_000_000,
                ),
            )
        })
        .collect()
}

fn tmp_files(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".tmp"))
        .collect()
}

/// Opens a backup file alone, read-only, as a restore would see it.
fn check_backup(path: &Path) -> i64 {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let ok: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ok, "ok");
    assert!(
        !conn
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap()
    );
    let n: i64 = conn
        .query_row("SELECT count(*) FROM marks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n % 2, 0, "each transaction wholly or not at all");
    n
}

fn assert_no_secret(bytes: &[u8]) {
    let secret_bytes = b"kohaku-test-secret-32-bytes-long";
    for needle in [&secret_bytes[..], TEST_SECRET.as_bytes()] {
        assert!(!bytes.windows(needle.len()).any(|w| w == needle));
    }
}

// Instance lock

#[test]
fn lock_in_one_process_excludes_a_second_holder() {
    let (_dir, data) = data_dir();
    let first = InstanceLock::acquire(&data).unwrap();
    assert!(matches!(InstanceLock::acquire(&data), Err(LockError::Held)));
    drop(first);
    InstanceLock::acquire(&data).unwrap();
}

// Backup to a file

#[test]
fn backup_while_the_server_runs() {
    let (dir, data) = data_dir();
    let conn = served(&data);
    let stop = Arc::new(AtomicBool::new(false));
    let writer = {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let mut i = 1000;
            while !stop.load(Ordering::SeqCst) {
                write_pair(&conn, i);
                i += 1;
            }
        })
    };
    let target_dir = TempDir::new();
    let target = target_dir.path().join("2026-09-23.db");
    backup_to_file(&data, &secret(), &target).unwrap();
    stop.store(true, Ordering::SeqCst);
    writer.join().unwrap();
    assert_eq!(mode(&target), 0o600);
    assert!(tmp_files(target_dir.path()).is_empty());
    assert!(tmp_files(dir.path()).is_empty());
    // A copy alone in a fresh directory needs no -wal or -shm.
    let alone = TempDir::new();
    let copy = alone.path().join("copy.db");
    fs::copy(&target, &copy).unwrap();
    assert!(check_backup(&copy) >= 100);
    assert_no_secret(&fs::read(&target).unwrap());
}

#[test]
fn existing_target_or_failure_leaves_nothing() {
    let (_dir, data) = data_dir();
    drop(served(&data));
    let targets = TempDir::new();
    let t = |name: &str| targets.path().join(name);
    fs::write(t("file.db"), b"keep").unwrap();
    fs::create_dir(t("dir.db")).unwrap();
    fs::write(t("real"), b"real").unwrap();
    symlink(t("real"), t("link.db")).unwrap();
    symlink(t("missing"), t("dangling.db")).unwrap();
    let before = listing(targets.path());
    for name in ["file.db", "dir.db", "link.db", "dangling.db"] {
        let error = backup_to_file(&data, &secret(), &t(name)).err().unwrap();
        assert!(matches!(error, BackupError::TargetExists(_)), "{name}");
        assert!(error.to_string().contains(name));
    }
    assert_eq!(listing(targets.path()), before);
    assert_eq!(fs::read(t("real")).unwrap(), b"real");
    assert!(!t("missing").exists());
    let error = backup_to_file(&data, &secret(), &t("nowhere/x.db"))
        .err()
        .unwrap();
    assert!(matches!(error, BackupError::Write(_)));
    assert!(!t("nowhere").exists());
}

#[test]
fn backup_refused_with_the_wrong_secret() {
    let (_dir, data) = data_dir();
    drop(served(&data));
    let targets = TempDir::new();
    let target = targets.path().join("b.db");
    let error = backup_to_file(&data, &other_secret(), &target)
        .err()
        .unwrap();
    assert!(matches!(
        error,
        BackupError::Keycheck(MigrateError::KeycheckMismatch)
    ));
    assert_eq!(
        error.to_string(),
        "KOHAKU_SECRET does not match the database"
    );
    assert_eq!(fs::read_dir(targets.path()).unwrap().count(), 0);
}

#[test]
fn keycheck_less_database_is_refused_by_backup() {
    let (_dir, data) = data_dir();
    drop(served(&data));
    open(&data.database(), Role::Command)
        .unwrap()
        .execute("DELETE FROM meta", [])
        .unwrap();
    let targets = TempDir::new();
    let error = backup_to_file(&data, &secret(), &targets.path().join("b.db"))
        .err()
        .unwrap();
    assert!(matches!(
        error,
        BackupError::Keycheck(MigrateError::KeycheckMissing)
    ));
    assert_eq!(fs::read_dir(targets.path()).unwrap().count(), 0);
}

#[test]
fn newer_schema_is_backed_up_at_its_version() {
    let (_dir, data) = data_dir();
    let newer = [
        MIGRATIONS[0],
        Migration {
            version: 2,
            name: "0002_future",
            sql: "CREATE TABLE future (id INTEGER PRIMARY KEY) STRICT;",
        },
    ];
    drop(open_and_prepare(&data, &secret(), &newer, NOW).unwrap());
    let targets = TempDir::new();
    let target = targets.path().join("b.db");
    backup_to_file(&data, &secret(), &target).unwrap();
    let conn = Connection::open_with_flags(&target, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(user_version(&conn).unwrap(), 2);
}

#[test]
fn missing_database_fails_creating_nothing() {
    let (dir, data) = data_dir();
    let targets = TempDir::new();
    let error = backup_to_file(&data, &secret(), &targets.path().join("b.db"))
        .err()
        .unwrap();
    assert!(matches!(error, BackupError::NoDatabase));
    assert_eq!(fs::read_dir(targets.path()).unwrap().count(), 0);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

// Backup to standard output

/// A writer that checks `/data/backups` on its first write and can fail after a limit.
struct Probe {
    backups: PathBuf,
    bytes: Vec<u8>,
    fail_after: Option<usize>,
    tmp_seen: Option<(usize, u32)>,
}

impl Write for Probe {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.tmp_seen.is_none() {
            let tmps = tmp_files(&self.backups);
            let tmp_mode = tmps.first().map_or(0, |n| mode(&self.backups.join(n)));
            self.tmp_seen = Some((tmps.len(), tmp_mode));
        }
        if let Some(limit) = self.fail_after
            && self.bytes.len() + buf.len() > limit
        {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "reader closed"));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn probe(data: &DataDir, fail_after: Option<usize>) -> Probe {
    Probe {
        backups: data.backups(),
        bytes: Vec::new(),
        fail_after,
        tmp_seen: None,
    }
}

#[test]
fn streaming_backup() {
    let (_dir, data) = data_dir();
    drop(served(&data));
    assert!(!data.backups().exists());
    let mut out = probe(&data, None);
    backup_to_writer(&data, &secret(), &mut out).unwrap();
    assert_eq!(mode(&data.backups()), 0o700);
    assert_eq!(out.tmp_seen, Some((1, 0o600)));
    assert!(tmp_files(&data.backups()).is_empty());
    assert!(out.bytes.starts_with(b"SQLite format 3\0"));
    let alone = TempDir::new();
    let copy = alone.path().join("copy.db");
    fs::write(&copy, &out.bytes).unwrap();
    assert_eq!(check_backup(&copy), 100);
    assert_no_secret(&out.bytes);
}

#[test]
fn reader_closing_early_fails_the_backup() {
    let (_dir, data) = data_dir();
    drop(served(&data));
    let mut out = probe(&data, Some(4096));
    let error = backup_to_writer(&data, &secret(), &mut out).err().unwrap();
    assert!(matches!(error, BackupError::Output(_)));
    assert!(error.to_string().contains("standard output"));
    assert!(tmp_files(&data.backups()).is_empty());
}

#[test]
fn no_byte_is_written_before_the_snapshot() {
    let (_dir, data) = data_dir();
    drop(served(&data));
    let mut out = probe(&data, None);
    assert!(backup_to_writer(&data, &other_secret(), &mut out).is_err());
    assert!(out.bytes.is_empty() && out.tmp_seen.is_none());
    assert!(!data.backups().exists());
}

// Listing backups

#[test]
fn files_are_listed_by_name() {
    let (_dir, data) = data_dir();
    let _lock = InstanceLock::acquire(&data).unwrap();
    assert!(list_backups(&data).unwrap().is_empty());
    assert!(!data.backups().exists());
    let backups = data.ensure_backups().unwrap();
    fs::write(backups.join("pre-migrate-v1-1790000000.db"), b"x").unwrap();
    fs::write(backups.join("2026-09-23.db"), b"x").unwrap();
    fs::create_dir(backups.join("sub")).unwrap();
    symlink(backups.join("2026-09-23.db"), backups.join("link.db")).unwrap();
    assert_eq!(
        list_backups(&data).unwrap(),
        ["2026-09-23.db", "pre-migrate-v1-1790000000.db"]
    );
}

// Restore

use kohaku::db::backup::{RestoreError, restore};

/// A backup (as `backup -` writes it) of a served database with `rows` pairs of marks
/// and `entries` audit entries.
fn backup_bytes(entries: usize) -> Vec<u8> {
    let (_dir, data) = data_dir();
    let conn = served(&data);
    for n in 0..entries {
        conn.execute(
            "INSERT INTO audit_log (actor_label, actor_id, action, target_type, target_id, time)
             VALUES ('maintainer', ?1, 'sample.update', 'sample', ?1, unixepoch())",
            [i64::try_from(n).unwrap() + 1],
        )
        .unwrap();
    }
    drop(conn);
    let mut out = Vec::new();
    backup_to_writer(&data, &secret(), &mut out).unwrap();
    out
}

/// Label, actor id, action, target type, target id.
type AuditRow = (String, Option<i64>, String, String, Option<i64>);

fn audit_rows(path: &Path) -> Vec<AuditRow> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    conn.prepare(
        "SELECT actor_label, actor_id, action, target_type, target_id FROM audit_log ORDER BY id",
    )
    .unwrap()
    .query_map([], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
    })
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

/// The live files' bytes (database, WAL, SHM), to show a refusal changed nothing.
fn live_files(data: &DataDir) -> Vec<Option<Vec<u8>>> {
    [data.database(), data.wal(), data.shm()]
        .iter()
        .map(|p| fs::read(p).ok())
        .collect()
}

/// A live database whose WAL holds rows the backup lacks.
fn live_with_wal(data: &DataDir) -> Connection {
    let conn = served(data);
    conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
    conn.execute("INSERT INTO marks (text) VALUES ('only-in-live')", [])
        .unwrap();
    conn
}

#[test]
fn restore_from_standard_input() {
    let bytes = backup_bytes(12);
    let (dir, data) = data_dir();
    let live = live_with_wal(&data);
    assert!(fs::metadata(data.wal()).unwrap().len() > 0);
    // `serve` is stopped: its connection is gone, its WAL left behind.
    std::mem::forget(live);
    restore(&data, &mut bytes.as_slice(), 1_900_000_000).unwrap();
    assert_eq!(mode(&data.database()), 0o600);
    assert!(tmp_files(dir.path()).is_empty());
    let conn = Connection::open(data.database()).unwrap();
    let live_only: i64 = conn
        .query_row(
            "SELECT count(*) FROM marks WHERE text = 'only-in-live'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(live_only, 0);
    let marks: i64 = conn
        .query_row("SELECT count(*) FROM marks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(marks, 100);
    drop(conn);
    let entries = audit_rows(&data.database());
    assert_eq!(entries.len(), 13, "the backup's 12 plus the restore");
    assert!(entries[..12].iter().all(|e| e.2 == "sample.update"));
    assert_eq!(
        entries[12],
        (
            "cli".to_owned(),
            None,
            "instance.restore".to_owned(),
            "instance".to_owned(),
            None
        )
    );
}

#[test]
fn bad_sources_change_nothing() {
    let good = backup_bytes(0);
    let mut corrupted = good.clone();
    for byte in &mut corrupted[4096..8192] {
        *byte ^= 0x5a;
    }
    let random: Vec<u8> = (0..20_000u32)
        .map(|n| (n.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    let sources = [
        Vec::new(),
        good[..good.len() / 2].to_vec(),
        random,
        corrupted,
    ];
    for source in sources {
        let (dir, data) = data_dir();
        let live = live_with_wal(&data);
        let before = live_files(&data);
        let error = restore(&data, &mut source.as_slice(), 1).unwrap_err();
        assert!(matches!(error, RestoreError::NotABackup), "{error:?}");
        assert!(error.to_string().contains("not a valid backup"));
        assert_eq!(live_files(&data), before);
        assert!(tmp_files(dir.path()).is_empty());
        drop(live);
    }
}

#[test]
fn version_zero_or_no_audit_log_is_refused() {
    let plain = TempDir::new();
    let zero = plain.path().join("zero.db");
    Connection::open(&zero)
        .unwrap()
        .execute_batch("CREATE TABLE x (a)")
        .unwrap();
    let no_audit = plain.path().join("no-audit.db");
    Connection::open(&no_audit)
        .unwrap()
        .execute_batch("CREATE TABLE x (a); PRAGMA user_version = 1;")
        .unwrap();
    for (path, expected) in [(zero, "NotABackup"), (no_audit, "Record")] {
        let (dir, data) = data_dir();
        let live = live_with_wal(&data);
        let before = live_files(&data);
        let error = restore(&data, &mut fs::File::open(&path).unwrap(), 1).unwrap_err();
        assert_eq!(format!("{error:?}"), expected);
        assert_eq!(live_files(&data), before);
        assert!(tmp_files(dir.path()).is_empty());
        let entries = audit_rows(&data.database());
        assert!(entries.is_empty(), "nothing recorded in the live database");
        drop(live);
    }
}

#[test]
fn restore_needs_the_lock() {
    let bytes = backup_bytes(0);
    let (_dir, data) = data_dir();
    drop(served(&data));
    let before = live_files(&data);
    let _held = InstanceLock::acquire(&data).unwrap();
    let error = restore(&data, &mut bytes.as_slice(), 1).unwrap_err();
    assert!(matches!(error, RestoreError::Lock(LockError::Held)));
    assert_eq!(live_files(&data), before);
}

#[test]
fn restore_entry_carries_no_file_name_or_content() {
    let (_dir, source_data) = data_dir();
    let conn = served(&source_data);
    conn.execute("INSERT INTO marks (text) VALUES ('Crash on start')", [])
        .unwrap();
    drop(conn);
    let files = TempDir::new();
    let path = files.path().join("victim@example.com-203.0.113.7.db");
    backup_to_file(&source_data, &secret(), &path).unwrap();
    let (_dir, data) = data_dir();
    restore(&data, &mut fs::File::open(&path).unwrap(), 1).unwrap();
    let conn = Connection::open(data.database()).unwrap();
    let text: String = conn
        .query_row(
            "SELECT group_concat(actor_label || action || target_type || coalesce(target_id, '') || time) FROM audit_log",
            [],
            |r| r.get(0),
        )
        .unwrap();
    for marker in ["victim", "203.0.113.7", "Crash", ".db"] {
        assert!(!text.contains(marker), "{text}");
    }
}

#[test]
fn restore_skips_the_keycheck_and_the_next_start_checks_it() {
    let bytes = backup_bytes(0);
    let (_dir, data) = data_dir();
    restore(&data, &mut bytes.as_slice(), 1).unwrap();
    let wrong = open_and_prepare(&data, &other_secret(), MIGRATIONS, NOW)
        .err()
        .unwrap();
    assert!(matches!(wrong, MigrateError::KeycheckMismatch));
    open_and_prepare(&data, &secret(), MIGRATIONS, NOW).unwrap();
}

#[test]
fn older_backup_is_migrated_at_the_next_start() {
    let bytes = backup_bytes(0);
    let (_dir, data) = data_dir();
    restore(&data, &mut bytes.as_slice(), 1).unwrap();
    let newer = [
        MIGRATIONS[0],
        Migration {
            version: 2,
            name: "0002_next",
            sql: "CREATE TABLE next (id INTEGER PRIMARY KEY) STRICT;",
        },
    ];
    let conn = open_and_prepare(&data, &secret(), &newer, NOW).unwrap();
    assert_eq!(user_version(&conn).unwrap(), 2);
    drop(conn);
    // The pre-migration copy restores and opens under the older runner.
    let copy = data.backups().join(format!("pre-migrate-v1-{NOW}.db"));
    let (_dir2, older) = data_dir();
    restore(&older, &mut fs::File::open(&copy).unwrap(), 2).unwrap();
    let conn = open_and_prepare(&older, &secret(), MIGRATIONS, NOW).unwrap();
    assert_eq!(user_version(&conn).unwrap(), 1);
}
