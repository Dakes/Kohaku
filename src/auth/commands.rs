//! `kohaku admin unlock` and `kohaku admin reset-password` (admin-auth: Account
//! commands; change admin-auth D11). Neither needs the instance lock: SQLite's busy
//! timeout orders their one transaction with a running `serve`.

use std::fmt;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

use super::AuthError;
use super::password::normalize_email;
use super::reset::issue_reset_token;
use crate::audit::{Action, Actor, Target, audit};
use crate::db::migrate::{MIGRATIONS, MigrateError, check_keycheck, user_version};
use crate::db::{DataDir, Role, open};
use crate::keys::InstanceSecret;
use crate::routing::urls::Urls;

#[derive(Debug)]
pub enum AccountCommandError {
    /// No database: `serve` never ran on this data directory.
    NoDatabase,
    /// The database is of another schema version than this binary's.
    Schema {
        database: u32,
        binary: u32,
    },
    Keycheck(MigrateError),
    NoAccount,
    Failed(AuthError),
}

impl From<rusqlite::Error> for AccountCommandError {
    fn from(error: rusqlite::Error) -> AccountCommandError {
        AccountCommandError::Failed(error.into())
    }
}

impl From<AuthError> for AccountCommandError {
    fn from(error: AuthError) -> AccountCommandError {
        AccountCommandError::Failed(error)
    }
}

impl fmt::Display for AccountCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AccountCommandError::NoDatabase => {
                f.write_str("there is no /data/kohaku.db; start `kohaku serve` first")
            }
            AccountCommandError::Schema { database, binary } => write!(
                f,
                "the database has schema version {database}, but this Kohaku expects {binary}; \
                 run the command with the release that `kohaku serve` runs, after it started"
            ),
            AccountCommandError::Keycheck(error) => error.fmt(f),
            AccountCommandError::NoAccount => f.write_str("no account has that email address"),
            AccountCommandError::Failed(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for AccountCommandError {}

/// The live database, only if it has this binary's schema and was made with `secret`.
fn open_current(
    data: &DataDir,
    secret: &InstanceSecret,
) -> Result<Connection, AccountCommandError> {
    let path = data.database();
    if !path.is_file() {
        return Err(AccountCommandError::NoDatabase);
    }
    let conn = open(&path, Role::Command)?;
    let database = user_version(&conn)?;
    let binary = MIGRATIONS.last().map_or(0, |m| m.version);
    if database != binary {
        return Err(AccountCommandError::Schema { database, binary });
    }
    check_keycheck(&conn, secret, None).map_err(AccountCommandError::Keycheck)?;
    Ok(conn)
}

/// Runs `work` in one write transaction with the account of `email`.
fn with_account<T>(
    data: &DataDir,
    secret: &InstanceSecret,
    email: &str,
    work: impl FnOnce(&rusqlite::Transaction<'_>, i64) -> Result<T, AccountCommandError>,
) -> Result<T, AccountCommandError> {
    let mut conn = open_current(data, secret)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let id: Option<i64> = tx
        .query_row(
            "SELECT id FROM users WHERE email = ?1",
            [normalize_email(email)],
            |row| row.get(0),
        )
        .optional()?;
    let id = id.ok_or(AccountCommandError::NoAccount)?;
    let value = work(&tx, id)?;
    tx.commit()?;
    Ok(value)
}

/// Ends the account's lock and resets both failure counts.
pub fn unlock(
    data: &DataDir,
    secret: &InstanceSecret,
    email: &str,
    now: i64,
) -> Result<(), AccountCommandError> {
    with_account(data, secret, email, |tx, id| {
        tx.execute(
            "UPDATE users SET locked_until = NULL, failed_logins = 0, device_failures = 0,
                 device_window_start = NULL
             WHERE id = ?1",
            [id],
        )?;
        audit(tx, Actor::Cli, Action::UserUnlock, Target::User(id), now)?;
        Ok(())
    })
}

/// A 1-hour reset link for the account, any role; mails nothing.
pub fn reset_password(
    data: &DataDir,
    secret: &InstanceSecret,
    urls: &Urls,
    email: &str,
    now: i64,
) -> Result<String, AccountCommandError> {
    with_account(data, secret, email, |tx, id| {
        let link = issue_reset_token(tx, urls, id, now)?;
        audit(tx, Actor::Cli, Action::UserResetLink, Target::User(id), now)?;
        Ok(link)
    })
}
