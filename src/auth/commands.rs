//! `kohaku admin unlock` and `kohaku admin reset-password` (admin-auth: Account
//! commands; change admin-auth D11). Neither needs the instance lock: SQLite's busy
//! timeout orders their one transaction with a running `serve`.

use std::fmt;

use rusqlite::{OptionalExtension, TransactionBehavior};

use super::AuthError;
use super::password::normalize_email;
use super::reset::issue_reset_token;
use crate::audit::{Action, Actor, Target, audit};
use crate::db::DataDir;
use crate::db::current::{OpenCurrentError, open_current};
use crate::keys::InstanceSecret;
use crate::routing::urls::Urls;

#[derive(Debug)]
pub enum AccountCommandError {
    Open(OpenCurrentError),
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
            AccountCommandError::Open(error) => error.fmt(f),
            AccountCommandError::NoAccount => f.write_str("no account has that email address"),
            AccountCommandError::Failed(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for AccountCommandError {}

/// Runs `work` in one write transaction with the account of `email`.
fn with_account<T>(
    data: &DataDir,
    secret: &InstanceSecret,
    email: &str,
    work: impl FnOnce(&rusqlite::Transaction<'_>, i64) -> Result<T, AccountCommandError>,
) -> Result<T, AccountCommandError> {
    let mut conn = open_current(data, secret).map_err(AccountCommandError::Open)?;
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
