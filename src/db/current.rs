//! The live database as a command beside `serve` opens it (`admin` and `project`
//! commands): never created, never migrated.

use std::fmt;

use rusqlite::Connection;

use super::migrate::{DbFailure, MIGRATIONS, MigrateError, check_keycheck, user_version};
use super::{DataDir, Role, open};
use crate::keys::InstanceSecret;

#[derive(Debug)]
pub enum OpenCurrentError {
    /// No database: `serve` never ran on this data directory.
    NoDatabase,
    /// The database is of another schema version than this binary's.
    Schema {
        database: u32,
        binary: u32,
    },
    Keycheck(MigrateError),
    Database(DbFailure),
}

impl From<rusqlite::Error> for OpenCurrentError {
    fn from(error: rusqlite::Error) -> OpenCurrentError {
        OpenCurrentError::Database(error.into())
    }
}

impl fmt::Display for OpenCurrentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenCurrentError::NoDatabase => {
                f.write_str("there is no /data/kohaku.db; start `kohaku serve` first")
            }
            OpenCurrentError::Schema { database, binary } => write!(
                f,
                "the database has schema version {database}, but this Kohaku expects {binary}; \
                 run the command with the release that `kohaku serve` runs, after it started"
            ),
            OpenCurrentError::Keycheck(error) => error.fmt(f),
            OpenCurrentError::Database(failure) => failure.fmt(f),
        }
    }
}

impl std::error::Error for OpenCurrentError {}

/// The live database, only if it has this binary's schema and was made with `secret`.
pub fn open_current(
    data: &DataDir,
    secret: &InstanceSecret,
) -> Result<Connection, OpenCurrentError> {
    let path = data.database();
    if !path.is_file() {
        return Err(OpenCurrentError::NoDatabase);
    }
    let conn = open(&path, Role::Command)?;
    let database = user_version(&conn)?;
    let binary = MIGRATIONS.last().map_or(0, |m| m.version);
    if database != binary {
        return Err(OpenCurrentError::Schema { database, binary });
    }
    check_keycheck(&conn, secret, None).map_err(OpenCurrentError::Keycheck)?;
    Ok(conn)
}
