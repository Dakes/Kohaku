//! `kohaku backup`, `kohaku restore` and `kohaku restore --list` (data-storage: Backup to a
//! file, Backup to standard output, Restore; operations: Listing backups; D9).

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::migrate::{MigrateError, check_keycheck, vacuum_into};
use super::paths::{DataDir, create_tmp, parent_dir, remove_if_present, sync_dir};
use super::{Role, open};
use crate::keys::InstanceSecret;

#[derive(Debug)]
pub enum BackupError {
    /// `/data/kohaku.db` does not exist or cannot be opened.
    NoDatabase,
    Keycheck(MigrateError),
    TargetExists(PathBuf),
    /// Writing the snapshot or the target failed.
    Write(io::Error),
    /// Standard output failed (a closed pipe, a full disk).
    Output(io::Error),
}

impl fmt::Display for BackupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackupError::NoDatabase => f.write_str("cannot open /data/kohaku.db"),
            BackupError::Keycheck(error) => error.fmt(f),
            BackupError::TargetExists(path) => write!(
                f,
                "{} already exists; a backup never replaces anything",
                path.display()
            ),
            BackupError::Write(error) => write!(f, "cannot write the backup: {error}"),
            BackupError::Output(error) => {
                write!(f, "cannot write the backup to standard output: {error}")
            }
        }
    }
}

impl std::error::Error for BackupError {}

/// Opens the existing database (never creating it) and checks its keycheck. No
/// downgrade guard: a newer database is backed up at its own version.
fn open_checked(data: &DataDir, secret: &InstanceSecret) -> Result<Connection, BackupError> {
    if !data.database().is_file() {
        return Err(BackupError::NoDatabase);
    }
    let conn = open(&data.database(), Role::Command).map_err(|_| BackupError::NoDatabase)?;
    check_keycheck(&conn, secret, None).map_err(BackupError::Keycheck)?;
    Ok(conn)
}

/// A snapshot in a new mode-0600 `*.tmp` in `dir`; the caller owns the file.
fn snapshot(conn: &Connection, dir: &Path) -> io::Result<PathBuf> {
    let (file, tmp) = create_tmp(dir, "backup")?;
    drop(file);
    let written = vacuum_into(conn, &tmp).and_then(|()| File::open(&tmp)?.sync_all());
    match written {
        Ok(()) => Ok(tmp),
        Err(error) => {
            let _ = remove_if_present(&tmp);
            Err(error)
        }
    }
}

/// `kohaku backup <file>`: publishes the snapshot with `link()`, which fails on any
/// entry at `target` (dangling symlinks and racing entries included).
pub fn backup_to_file(
    data: &DataDir,
    secret: &InstanceSecret,
    target: &Path,
) -> Result<(), BackupError> {
    if fs::symlink_metadata(target).is_ok() {
        return Err(BackupError::TargetExists(target.to_path_buf()));
    }
    let conn = open_checked(data, secret)?;
    let dir = parent_dir(target);
    let tmp = snapshot(&conn, dir).map_err(BackupError::Write)?;
    drop(conn);
    let published = fs::hard_link(&tmp, target);
    let removed = remove_if_present(&tmp);
    match published {
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            return Err(BackupError::TargetExists(target.to_path_buf()));
        }
        Err(error) => return Err(BackupError::Write(error)),
        Ok(()) => {}
    }
    removed
        .and_then(|()| sync_dir(dir))
        .map_err(BackupError::Write)
}

/// `kohaku backup -`: the snapshot goes through a `*.tmp` in `/data/backups`, removed
/// whether or not copying to `out` succeeds. Nothing reaches `out` before the
/// snapshot is complete.
pub fn backup_to_writer(
    data: &DataDir,
    secret: &InstanceSecret,
    out: &mut dyn Write,
) -> Result<(), BackupError> {
    let conn = open_checked(data, secret)?;
    let dir = data.ensure_backups().map_err(BackupError::Write)?;
    let tmp = snapshot(&conn, &dir).map_err(BackupError::Write)?;
    drop(conn);
    let copied = File::open(&tmp)
        .map_err(BackupError::Write)
        .and_then(|mut file| {
            io::copy(&mut file, out)
                .and_then(|_| out.flush())
                .map_err(BackupError::Output)
        });
    let removed = remove_if_present(&tmp).map_err(BackupError::Write);
    copied.and(removed)
}

/// `kohaku restore --list`: names of the regular files in `/data/backups`, in
/// ascending byte order; none for a missing directory, which is not created.
pub fn list_backups(data: &DataDir) -> io::Result<Vec<OsString>> {
    let entries = match fs::read_dir(data.backups()) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            names.push(entry.file_name());
        }
    }
    names.sort_by(|a, b| a.as_encoded_bytes().cmp(b.as_encoded_bytes()));
    Ok(names)
}

/// Why a restore refused or failed; the live database is unchanged in every case.
#[derive(Debug)]
pub enum RestoreError {
    Lock(super::lock::LockError),
    /// Reading the source or writing the copy failed.
    Io(io::Error),
    /// Empty, not SQLite, damaged, or schema version 0.
    NotABackup,
    /// The backup cannot take the restore entry (it has no audit log).
    Record,
}

impl fmt::Display for RestoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RestoreError::Lock(error) => error.fmt(f),
            RestoreError::Io(error) => write!(f, "cannot restore: {error}"),
            RestoreError::NotABackup => {
                f.write_str("the source is not a valid backup; the database is unchanged")
            }
            RestoreError::Record => f.write_str(
                "the backup cannot record the restore in its audit log; the database is unchanged",
            ),
        }
    }
}

impl std::error::Error for RestoreError {}

/// `kohaku restore <file>` and `kohaku restore -`: replaces `/data/kohaku.db` with the
/// backup read from `source`, holding the instance lock. No keycheck, no migration: the
/// next `serve` does both.
pub fn restore(data: &DataDir, source: &mut dyn io::Read, now: i64) -> Result<(), RestoreError> {
    let _lock = super::lock::InstanceLock::acquire(data).map_err(RestoreError::Lock)?;
    let (mut file, tmp) = create_tmp(data.root(), "restore").map_err(RestoreError::Io)?;
    let result = (|| {
        io::copy(source, &mut file).map_err(RestoreError::Io)?;
        file.sync_all().map_err(RestoreError::Io)?;
        drop(file);
        check_and_record(&tmp, now)?;
        File::open(&tmp)
            .and_then(|f| f.sync_all())
            .map_err(RestoreError::Io)?;
        remove_if_present(&data.wal()).map_err(RestoreError::Io)?;
        remove_if_present(&data.shm()).map_err(RestoreError::Io)?;
        fs::rename(&tmp, data.database()).map_err(RestoreError::Io)?;
        sync_dir(data.root()).map_err(RestoreError::Io)
    })();
    if result.is_err() {
        let _ = remove_if_present(&tmp);
    }
    result
}

/// Checks the candidate and records `instance.restore` in it, without side files: a
/// memory journal, since a crash discards the candidate anyway.
fn check_and_record(path: &Path, now: i64) -> Result<(), RestoreError> {
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| RestoreError::NotABackup)?;
    let checked = (|| -> rusqlite::Result<bool> {
        conn.query_row("PRAGMA journal_mode=MEMORY", [], |_| Ok(()))?;
        let integrity: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        let version = super::migrate::user_version(&conn)?;
        Ok(integrity == "ok" && version >= 1)
    })();
    if !matches!(checked, Ok(true)) {
        return Err(RestoreError::NotABackup);
    }
    let mut conn = conn;
    let recorded = (|| -> rusqlite::Result<()> {
        let tx = conn.transaction()?;
        crate::audit::audit(
            &tx,
            crate::audit::Actor::Cli,
            crate::audit::Action::InstanceRestore,
            crate::audit::Target::Instance,
            now,
        )?;
        tx.commit()
    })();
    recorded.map_err(|_| RestoreError::Record)?;
    conn.close().map_err(|_| RestoreError::Record)
}
