//! Forward-only migrations at startup (data-storage: Schema version and migrations at
//! startup, Downgrade guard, Pre-migration copy; change foundation D6, D8).

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

use super::paths::{DataDir, create_tmp, remove_if_present, sync_dir};
use crate::keys::InstanceSecret;

/// One embedded migration: `migrations/NNNN_<change>.sql`, `NNNN` its resulting
/// `user_version`. Migration files hold no transaction statements.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

/// Every migration, in order. A released one is never edited, renumbered or removed.
pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "0001_foundation",
    sql: include_str!("../../migrations/0001_foundation.sql"),
}];

/// Pre-migration copies kept after a new one is written.
const PRE_MIGRATE_KEPT: usize = 2;

#[derive(Debug)]
pub enum MigrateError {
    /// `/data/kohaku.db` cannot be created or opened.
    Open(io::ErrorKind),
    /// The database is newer than this binary.
    Downgrade {
        database: u32,
        binary: u32,
    },
    /// Version 0 but tables present: some other SQLite file.
    NotKohaku,
    KeycheckMismatch,
    /// A database without its keycheck, refused by every command but `serve`.
    KeycheckMissing,
    PreMigrationCopy(io::Error),
    ForeignKeyViolation {
        migration: &'static str,
    },
    Statement {
        migration: &'static str,
        cause: DbFailure,
    },
    Database(DbFailure),
}

/// A SQLite failure reduced to its result code; the message text (which may quote
/// SQL or values) never leaves this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DbFailure(pub Option<rusqlite::ErrorCode>);

impl From<rusqlite::Error> for DbFailure {
    fn from(error: rusqlite::Error) -> DbFailure {
        DbFailure(error.sqlite_error_code())
    }
}

impl fmt::Display for DbFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(code) => write!(f, "database error ({code:?})"),
            None => f.write_str("database error"),
        }
    }
}

impl From<rusqlite::Error> for MigrateError {
    fn from(error: rusqlite::Error) -> MigrateError {
        MigrateError::Database(error.into())
    }
}

impl fmt::Display for MigrateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MigrateError::Downgrade { database, binary } => write!(
                f,
                "the database has schema version {database}, but this Kohaku knows only up to \
                 {binary}: the binary is older than the database; run a newer release or restore \
                 a pre-migration copy (see the README)"
            ),
            MigrateError::Open(kind) => write!(f, "cannot open /data/kohaku.db ({kind})"),
            MigrateError::NotKohaku => f.write_str("kohaku.db is not a Kohaku database"),
            MigrateError::KeycheckMismatch => {
                f.write_str("KOHAKU_SECRET does not match the database")
            }
            MigrateError::KeycheckMissing => f.write_str(
                "the database has no keycheck: it was never completed by `kohaku serve`",
            ),
            MigrateError::PreMigrationCopy(error) => write!(
                f,
                "cannot write the pre-migration copy to /data/backups ({}); no migration applied",
                error.kind()
            ),
            MigrateError::ForeignKeyViolation { migration } => write!(
                f,
                "migration {migration} left a foreign-key violation and was rolled back"
            ),
            MigrateError::Statement { migration, cause } => {
                write!(
                    f,
                    "migration {migration} failed and was rolled back: {cause}"
                )
            }
            MigrateError::Database(cause) => cause.fmt(f),
        }
    }
}

impl std::error::Error for MigrateError {}

pub fn user_version(conn: &Connection) -> rusqlite::Result<u32> {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
}

fn newest(migrations: &[Migration]) -> u32 {
    migrations.last().map_or(0, |m| m.version)
}

/// Whether the file holds any schema object.
fn has_schema(conn: &Connection) -> rusqlite::Result<bool> {
    conn.query_row("SELECT EXISTS (SELECT 1 FROM sqlite_schema)", [], |row| {
        row.get(0)
    })
}

/// The stored keycheck, `None` when `meta` or its row is missing.
fn stored_keycheck(conn: &Connection) -> rusqlite::Result<Option<Vec<u8>>> {
    if !stored_meta_exists(conn)? {
        return Ok(None);
    }
    conn.query_row("SELECT keycheck FROM meta WHERE id = 1", [], |row| {
        row.get(0)
    })
    .optional()
}

fn insert_keycheck(conn: &Connection, secret: &InstanceSecret, now: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO meta (id, keycheck, created_at) VALUES (1, ?1, ?2)",
        (secret.keycheck().as_slice(), now),
    )
    .map(drop)
}

/// The keycheck step of every command but `restore` (configuration: Database
/// keycheck). `serve` completes a missing keycheck; other commands refuse it.
pub fn check_keycheck(
    conn: &Connection,
    secret: &InstanceSecret,
    complete_missing: Option<i64>,
) -> Result<(), MigrateError> {
    match stored_keycheck(conn)? {
        Some(stored) if secret.matches_keycheck(&stored) => Ok(()),
        Some(_) => Err(MigrateError::KeycheckMismatch),
        None => match complete_missing {
            Some(now) if stored_meta_exists(conn)? => {
                insert_keycheck(conn, secret, now)?;
                Ok(())
            }
            _ => Err(MigrateError::KeycheckMissing),
        },
    }
}

fn stored_meta_exists(conn: &Connection) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'meta')",
        [],
        |row| row.get(0),
    )
}

/// Opens `/data/kohaku.db` as the writer, creating it (mode 0600) when missing, and
/// brings it to the newest migration. `serve`'s database step.
pub fn open_and_prepare(
    data: &DataDir,
    secret: &InstanceSecret,
    migrations: &[Migration],
    now: i64,
) -> Result<Connection, MigrateError> {
    let path = data.database();
    super::create_file(&path).map_err(|error| MigrateError::Open(error.kind()))?;
    let mut conn = super::open(&path, super::Role::Writer)?;
    prepare(&mut conn, data, secret, migrations, now)?;
    Ok(conn)
}

/// Brings the writer's database to the newest migration, in the order of design §3:
/// downgrade guard, not-Kohaku refusal, keycheck, pre-migration copy, migrations.
/// Runs under the instance lock, before anything listens.
pub fn prepare(
    conn: &mut Connection,
    data: &DataDir,
    secret: &InstanceSecret,
    migrations: &[Migration],
    now: i64,
) -> Result<(), MigrateError> {
    debug_assert!(
        migrations
            .iter()
            .enumerate()
            .all(|(i, m)| usize::try_from(m.version).is_ok_and(|v| v == i + 1)),
        "migrations are numbered 1, 2, 3, ... without gaps"
    );
    let version = user_version(conn)?;
    let binary = newest(migrations);
    if version > binary {
        return Err(MigrateError::Downgrade {
            database: version,
            binary,
        });
    }
    if version == 0 {
        if has_schema(conn)? {
            return Err(MigrateError::NotKohaku);
        }
    } else {
        check_keycheck(conn, secret, Some(now))?;
        if version < binary {
            pre_migration_copy(conn, data, version, now).map_err(MigrateError::PreMigrationCopy)?;
        }
    }
    for migration in migrations.iter().filter(|m| m.version > version) {
        apply(conn, migration, secret, now)?;
    }
    Ok(())
}

/// One migration and its version bump in one transaction with foreign keys off,
/// committed only when the foreign-key check finds nothing. A new database gets its
/// keycheck in migration 1's transaction.
fn apply(
    conn: &mut Connection,
    migration: &Migration,
    secret: &InstanceSecret,
    now: i64,
) -> Result<(), MigrateError> {
    // A no-op inside a transaction, so it is set before BEGIN.
    conn.pragma_update(None, "foreign_keys", false)?;
    let result = apply_in_transaction(conn, migration, secret, now);
    conn.pragma_update(None, "foreign_keys", true)?;
    result
}

fn apply_in_transaction(
    conn: &mut Connection,
    migration: &Migration,
    secret: &InstanceSecret,
    now: i64,
) -> Result<(), MigrateError> {
    let statement_failed = |error: rusqlite::Error| MigrateError::Statement {
        migration: migration.name,
        cause: error.into(),
    };
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch(migration.sql).map_err(statement_failed)?;
    if migration.version == 1 {
        insert_keycheck(&tx, secret, now).map_err(statement_failed)?;
    }
    tx.pragma_update(None, "user_version", migration.version)?;
    let violation = tx.prepare("PRAGMA foreign_key_check")?.exists([])?;
    if violation {
        return Err(MigrateError::ForeignKeyViolation {
            migration: migration.name,
        });
    }
    tx.commit()?;
    Ok(())
}

/// `VACUUM INTO` a new `.tmp` in `/data/backups`, flushed, then renamed to
/// `pre-migrate-v{old}-{unixtime}.db`; only then are older copies pruned.
fn pre_migration_copy(conn: &Connection, data: &DataDir, old: u32, now: i64) -> io::Result<()> {
    let dir = data.ensure_backups()?;
    let (file, tmp) = create_tmp(&dir, "pre-migrate")?;
    drop(file);
    let written = vacuum_into(conn, &tmp).and_then(|()| {
        fs::File::open(&tmp)?.sync_all()?;
        let target = dir.join(format!("pre-migrate-v{old}-{now}.db"));
        fs::rename(&tmp, &target)?;
        sync_dir(&dir)
    });
    if let Err(error) = written {
        // The copy failed; its partial file must not look like a copy.
        let _ = remove_if_present(&tmp);
        return Err(error);
    }
    prune_pre_migration_copies(&dir)
}

/// Writes a consistent copy of `conn`'s database into the empty file at `path`.
pub fn vacuum_into(conn: &Connection, path: &Path) -> io::Result<()> {
    let path = path
        .to_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path is not UTF-8"))?;
    conn.execute("VACUUM INTO ?1", [path])
        .map(drop)
        .map_err(|error| io::Error::other(DbFailure::from(error).to_string()))
}

/// The `{unixtime}` of a `pre-migrate-v{digits}-{digits}.db` name.
fn pre_migration_time(name: &OsString) -> Option<u64> {
    let name = name.to_str()?;
    let rest = name.strip_prefix("pre-migrate-v")?.strip_suffix(".db")?;
    let (version, time) = rest.split_once('-')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(version) || !digits(time) {
        return None;
    }
    time.parse().ok()
}

fn prune_pre_migration_copies(dir: &Path) -> io::Result<()> {
    let mut copies = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        if let Some(time) = pre_migration_time(&name) {
            copies.push((time, name));
        }
    }
    copies.sort();
    let excess = copies.len().saturating_sub(PRE_MIGRATE_KEPT);
    for (_, name) in &copies[..excess] {
        remove_if_present(&dir.join(name))?;
    }
    if excess > 0 {
        sync_dir(dir)?;
    }
    Ok(())
}
