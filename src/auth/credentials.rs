//! The one credential check behind login and re-authentication: password, second
//! factor and lockout, each decided on the writer (admin-auth: Login, Lockout,
//! Re-authentication; change admin-auth D5–D7).

use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::password::{PASSWORD_MAX, dummy_hash, verify_password};
use super::session::is_known_device;
use super::totp::{Seed, is_code, matching_step};
use super::{AuthError, Role, mail, recovery};
use crate::db::consume_one;
use crate::keys::InstanceSecret;
use crate::mail::outbox::{NewMail, Recipient, Wakeup, enqueue};
use crate::routing::AppState;

/// Failures without a device cookie that lock the account...
pub const LOCK_AFTER: i64 = 10;
/// ...for this long.
pub const LOCK_SECONDS: i64 = 15 * 60;
/// Failures with a device cookie count in a window this long from the first...
pub const DEVICE_WINDOW_SECONDS: i64 = 60 * 60;
/// ...and past this many, such attempts fail until the window ends.
pub const DEVICE_FAILURES_MAX: i64 = 20;
/// At most one lockout mail per account in this time.
pub const LOCKOUT_MAIL_SECONDS: i64 = 24 * 60 * 60;

/// Longest code field: a recovery code with its hyphens and some spaces.
pub const CODE_MAX: usize = 32;

/// Which account an attempt is for.
pub enum Lookup {
    /// A normalized email (login).
    Email(String),
    /// The signed-in account (re-authentication).
    Id(i64),
}

/// One password check, as submitted.
pub struct Attempt {
    pub account: Lookup,
    pub password: String,
    pub code: String,
    /// SHA-256 of the request's device cookie, if it had a well-formed one.
    pub device: Option<[u8; 32]>,
}

/// An account as the check reads it.
#[derive(Clone)]
pub struct Account {
    pub id: i64,
    pub email: String,
    pub role: Role,
    disabled: bool,
    password_hash: Option<String>,
    totp_nonce: Option<Vec<u8>>,
    locked_until: Option<i64>,
    device_failures: i64,
    device_window_start: Option<i64>,
}

const ACCOUNT_COLUMNS: &str = "id, email, role, disabled, password_hash, totp_nonce, \
     locked_until, device_failures, device_window_start";

fn account_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    Ok(Account {
        id: row.get(0)?,
        email: row.get(1)?,
        role: row.get(2)?,
        disabled: row.get(3)?,
        password_hash: row.get(4)?,
        totp_nonce: row.get(5)?,
        locked_until: row.get(6)?,
        device_failures: row.get(7)?,
        device_window_start: row.get(8)?,
    })
}

fn find(conn: &Connection, lookup: &Lookup) -> rusqlite::Result<Option<Account>> {
    let (column, value): (&str, rusqlite::types::Value) = match lookup {
        Lookup::Email(email) => ("email", email.clone().into()),
        Lookup::Id(id) => ("id", (*id).into()),
    };
    conn.query_row(
        &format!("SELECT {ACCOUNT_COLUMNS} FROM users WHERE {column} = ?1"),
        [value],
        account_row,
    )
    .optional()
}

impl Account {
    /// Whether this request may not sign in at `now` whatever it submits: the lock for
    /// browsers without a device cookie, the window cap for those with one.
    fn blocked(&self, known_device: bool, now: i64) -> bool {
        if known_device {
            self.device_window_start
                .is_some_and(|start| start + DEVICE_WINDOW_SECONDS > now)
                && self.device_failures >= DEVICE_FAILURES_MAX
        } else {
            self.locked_until.is_some_and(|until| until > now)
        }
    }

    /// The hash to check the password against, if this request may try it.
    fn usable_hash(&self, known_device: bool, now: i64) -> Option<&str> {
        match &self.password_hash {
            Some(hash) if !self.disabled && !self.blocked(known_device, now) => Some(hash),
            _ => None,
        }
    }
}

/// How a check ended.
pub enum Outcome<T> {
    /// Every factor passed; the success work ran in the same transaction.
    Success(T),
    /// The generic failure, counted toward the lockout unless the account is blocked.
    Failure,
    /// The password was right, but the account must enroll a second factor first.
    TwoFactorRequired,
    /// No hashing permit within 2 s: 503, not counted.
    Busy,
}

/// Runs `attempt`: argon2id exactly once (the account's hash or the dummy), then one
/// write transaction deciding the lock, the code and the counters, running `success`
/// in it only when every factor passed.
pub async fn check<T, F>(
    app: &AppState,
    attempt: Attempt,
    success: F,
) -> Result<Outcome<T>, AuthError>
where
    F: FnOnce(&Transaction<'_>, &Account, i64) -> Result<T, AuthError> + Send + 'static,
    T: Send + 'static,
{
    let now = app.clock.unix();
    let Attempt {
        account: lookup,
        password,
        code,
        device,
    } = attempt;
    let (account, known_device) = app
        .db
        .read(move |conn| {
            let account = find(conn, &lookup)?;
            let known = match (&account, device) {
                (Some(account), Some(hash)) => is_known_device(conn, account.id, &hash, now)?,
                _ => false,
            };
            Ok::<_, rusqlite::Error>((account, known))
        })
        .await?;
    let Some(permit) = app.permits.hashing(known_device).await else {
        return Ok(Outcome::Busy);
    };
    let oversize = password.chars().count() > PASSWORD_MAX || code.chars().count() > CODE_MAX;
    let target = account
        .as_ref()
        .and_then(|a| a.usable_hash(known_device, now))
        .filter(|_| !oversize)
        .map(str::to_owned);
    let dummy = dummy_hash()?;
    let checked = target.clone();
    app.password_checks
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let matched = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        // An oversize password is checked as empty: hashing input stays capped.
        let input = if oversize { "" } else { password.as_str() };
        verify_password(input, checked.as_deref().unwrap_or(dummy)) && checked.is_some()
    })
    .await
    .expect("password verification does not panic");
    let Some(account) = account else {
        return Ok(Outcome::Failure);
    };
    let context = Context {
        secret: Arc::clone(&app.secret),
        wakeup: app.outbox.clone(),
        origin: app.urls.main_origin().to_owned(),
    };
    app.db
        .write(move |tx| {
            decide(
                tx,
                &context,
                account.id,
                Verified {
                    hash: target.filter(|_| matched),
                    known_device,
                    code,
                },
                now,
                success,
            )
        })
        .await
}

struct Context {
    secret: Arc<InstanceSecret>,
    wakeup: Wakeup,
    origin: String,
}

/// What was checked outside the transaction.
struct Verified {
    /// The hash the password matched, `None` when it did not.
    hash: Option<String>,
    known_device: bool,
    code: String,
}

fn decide<T, F>(
    tx: &Transaction<'_>,
    context: &Context,
    id: i64,
    verified: Verified,
    now: i64,
    success: F,
) -> Result<Outcome<T>, AuthError>
where
    F: FnOnce(&Transaction<'_>, &Account, i64) -> Result<T, AuthError>,
{
    let Some(account) = find(tx, &Lookup::Id(id))? else {
        return Ok(Outcome::Failure);
    };
    let known_device = verified.known_device;
    if account.blocked(known_device, now) {
        return Ok(Outcome::Failure);
    }
    // The password must match the hash stored now, not one changed meanwhile.
    let password_ok =
        verified.hash.is_some() && verified.hash == account.password_hash && !account.disabled;
    if !password_ok {
        count_failure(tx, context, &account, known_device, now)?;
        return Ok(Outcome::Failure);
    }
    let second_factor = match &account.totp_nonce {
        None if account.role.requires_two_factor() => return Ok(Outcome::TwoFactorRequired),
        None => true,
        Some(nonce) => {
            let seed = Seed::derive(&context.secret, account.id, nonce);
            code_accepted(tx, account.id, &seed, &verified.code, now)?
        }
    };
    if !second_factor {
        count_failure(tx, context, &account, known_device, now)?;
        return Ok(Outcome::Failure);
    }
    tx.execute(
        "UPDATE users SET failed_logins = 0, device_failures = 0, device_window_start = NULL
         WHERE id = ?1",
        [account.id],
    )?;
    success(tx, &account, now).map(Outcome::Success)
}

/// A TOTP code of a later step than the last accepted one, recorded by one conditional
/// write, or an unused recovery code, deleted by one.
pub fn code_accepted(
    tx: &Transaction<'_>,
    user_id: i64,
    seed: &Seed,
    code: &str,
    now: i64,
) -> rusqlite::Result<bool> {
    let code = code.trim_matches(|c: char| c.is_ascii_whitespace());
    if is_code(code) {
        let Some(step) = matching_step(seed, code, now) else {
            return Ok(false);
        };
        return consume_one(
            tx,
            "UPDATE users SET totp_last_step = ?2
             WHERE id = ?1 AND (totp_last_step IS NULL OR totp_last_step < ?2)",
            params![user_id, step],
        );
    }
    match recovery::normalize(code) {
        Some(code) => recovery::consume(tx, user_id, &code),
        None => Ok(false),
    }
}

/// Counts one failure (D7): toward the lock without a device cookie, in the device
/// window with one; nothing while the account is blocked for this request.
fn count_failure(
    tx: &Transaction<'_>,
    context: &Context,
    account: &Account,
    known_device: bool,
    now: i64,
) -> Result<(), AuthError> {
    if known_device {
        tx.execute(
            "UPDATE users SET
                 device_failures = CASE WHEN device_window_start IS NULL
                     OR device_window_start + ?3 <= ?2 THEN 1 ELSE device_failures + 1 END,
                 device_window_start = CASE WHEN device_window_start IS NULL
                     OR device_window_start + ?3 <= ?2 THEN ?2 ELSE device_window_start END
             WHERE id = ?1
               AND NOT (coalesce(device_window_start + ?3 > ?2, 0) AND device_failures >= ?4)",
            params![account.id, now, DEVICE_WINDOW_SECONDS, DEVICE_FAILURES_MAX],
        )?;
        return Ok(());
    }
    let started: Option<(bool, bool)> = tx
        .query_row(
            "UPDATE users SET
                 failed_logins = CASE WHEN failed_logins + 1 >= ?3 THEN 0
                     ELSE failed_logins + 1 END,
                 locked_until = CASE WHEN failed_logins + 1 >= ?3 THEN ?2 + ?4
                     ELSE locked_until END,
                 lockout_mailed_at = CASE WHEN failed_logins + 1 >= ?3
                     AND (lockout_mailed_at IS NULL OR lockout_mailed_at <= ?2 - ?5)
                     THEN ?2 ELSE lockout_mailed_at END
             WHERE id = ?1 AND (locked_until IS NULL OR locked_until <= ?2)
             RETURNING coalesce(locked_until = ?2 + ?4, 0), coalesce(lockout_mailed_at = ?2, 0)",
            params![
                account.id,
                now,
                LOCK_AFTER,
                LOCK_SECONDS,
                LOCKOUT_MAIL_SECONDS
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((true, true)) = started {
        let text = mail::lockout(&context.origin, now);
        enqueue(
            tx,
            &context.wakeup,
            NewMail {
                kind: &mail::LOCKOUT,
                recipient: Recipient::User(account.id),
                subject: text.subject,
                body: &text.body,
                placeholder: false,
            },
            now,
        )?;
    }
    Ok(())
}
