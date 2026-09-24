//! The persistent outbox and its worker (mail-outbox; change foundation D17).

use std::sync::Arc;
use std::time::Duration;

use rusqlite::{OptionalExtension, Transaction, params};
use tokio::sync::{Notify, watch};

use super::{Failure, FailureClass, Mailer, Outgoing, build_message, subject, validate_recipient};
use crate::config::SenderAddress;
use crate::db::Db;
use crate::db::migrate::DbFailure;
use crate::time::now_unix;

/// How long an attempt may run before it is abandoned as failed.
pub const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(60);

/// Delay before the next attempt after the 1st, 2nd, 3rd and 4th failure; then the
/// last one after each further failure.
pub const RETRY_DELAYS: [i64; 4] = [60, 5 * 60, 30 * 60, 2 * 60 * 60];

/// A row is given up at the latest this long after it was queued.
pub const MAX_AGE: i64 = 24 * 60 * 60;

/// Pause after the worker itself failed to read the outbox (a locked database).
const WORKER_ERROR_PAUSE: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// Warns a user about their account's security; attempted before any normal row.
    Security,
    Normal,
}

impl Priority {
    fn as_str(self) -> &'static str {
        match self {
            Priority::Security => "security",
            Priority::Normal => "normal",
        }
    }
}

/// A kind of mail, declared by the change that sends it.
pub struct MailKind {
    /// Stored in the row; lowercase letters and `_`.
    pub name: &'static str,
    pub priority: Priority,
    /// Carries a token or one-time code: deleted when sent or given up.
    pub token: bool,
    /// Seconds from queuing to expiry; no later than the token or code it carries.
    pub lifetime: i64,
    /// Stops the row's token or code from being accepted, in the give-up transaction.
    pub give_up: Option<fn(&Transaction<'_>, i64) -> rusqlite::Result<()>>,
}

/// Every mail kind; later changes add theirs.
pub const KINDS: &[&MailKind] = &[
    &crate::auth::mail::RESET_LINK,
    &crate::auth::mail::LOCKOUT,
    &crate::auth::mail::PASSWORD_CHANGED,
];

/// Wakes the worker when a row is queued.
#[derive(Debug, Clone, Default)]
pub struct Wakeup(Arc<Notify>);

impl Wakeup {
    pub fn wake(&self) {
        self.0.notify_one();
    }
}

/// Who a row goes to.
pub enum Recipient {
    Address(String),
    /// An account, delivered to its address at delivery time; its rows go with it.
    User(i64),
}

pub struct NewMail<'a> {
    pub kind: &'static MailKind,
    pub recipient: Recipient,
    pub subject: &'a str,
    pub body: &'a str,
    /// Queued only so a request does the same work whether or not an account exists;
    /// deleted unsent.
    pub placeholder: bool,
}

#[derive(Debug)]
pub enum EnqueueError {
    InvalidRecipient,
    Database(rusqlite::Error),
}

impl From<rusqlite::Error> for EnqueueError {
    fn from(error: rusqlite::Error) -> EnqueueError {
        EnqueueError::Database(error)
    }
}

/// Queues `mail` in the caller's write transaction, due at once, and wakes the worker.
/// The worker reads through the same writer, so it sees the row only once committed.
pub fn enqueue(
    tx: &Transaction<'_>,
    wakeup: &Wakeup,
    mail: NewMail<'_>,
    now: i64,
) -> Result<i64, EnqueueError> {
    let (user_id, address) = match mail.recipient {
        Recipient::Address(address) => {
            validate_recipient(&address).map_err(|_| EnqueueError::InvalidRecipient)?;
            (None, Some(address))
        }
        Recipient::User(id) => (Some(id), None),
    };
    let kind = mail.kind;
    let id = tx.query_row(
        "INSERT INTO outbox (kind, user_id, address, subject, body, priority, token,
             placeholder, next_attempt_at, queued_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10)
         RETURNING id",
        params![
            kind.name,
            user_id,
            address,
            subject(mail.subject),
            mail.body,
            kind.priority.as_str(),
            kind.token,
            mail.placeholder,
            now,
            now + kind.lifetime,
        ],
        |row| row.get(0),
    )?;
    wakeup.wake();
    Ok(id)
}

/// The earlier of queue time plus 24 h and the row's expiry.
pub fn deadline(queued_at: i64, expires_at: i64) -> i64 {
    (queued_at + MAX_AGE).min(expires_at)
}

/// The next attempt after the `failures`-th failure ending at `ended_at`; `None` when
/// it would start at or after the deadline, so the row is given up.
pub fn next_attempt(failures: u32, ended_at: i64, deadline: i64) -> Option<i64> {
    let index = usize::try_from(failures.saturating_sub(1)).unwrap_or(usize::MAX);
    let delay = RETRY_DELAYS[index.min(RETRY_DELAYS.len() - 1)];
    let next = ended_at + delay;
    (next < deadline).then_some(next)
}

fn find_kind(kinds: &[&'static MailKind], name: &str) -> Option<&'static MailKind> {
    kinds.iter().copied().find(|kind| kind.name == name)
}

/// Gives up an unsent row: its kind's give-up action invalidates its token, then a
/// token-bearing row is deleted and any other marked, all in `tx`. The one function
/// every removal of an unsent row goes through.
pub fn give_up(tx: &Transaction<'_>, kinds: &[&'static MailKind], id: i64) -> rusqlite::Result<()> {
    let row: Option<(String, bool)> = tx
        .query_row(
            "SELECT kind, token FROM outbox WHERE id = ?1 AND outcome IS NULL",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((kind, token)) = row else {
        return Ok(());
    };
    if let Some(action) = find_kind(kinds, &kind).and_then(|k| k.give_up) {
        action(tx, id)?;
    }
    if token {
        tx.execute("DELETE FROM outbox WHERE id = ?1", [id])?;
    } else {
        tx.execute(
            "UPDATE outbox SET outcome = 'given_up' WHERE id = ?1 AND outcome IS NULL",
            [id],
        )?;
    }
    Ok(())
}

/// Gives up every unsent row whose deadline is not after `now` (worker and retention).
pub fn give_up_expired(
    tx: &Transaction<'_>,
    kinds: &[&'static MailKind],
    now: i64,
) -> rusqlite::Result<usize> {
    let ids: Vec<i64> = tx
        .prepare(
            "SELECT id FROM outbox
             WHERE outcome IS NULL AND min(queued_at + ?1, expires_at) <= ?2",
        )?
        .query_map(params![MAX_AGE, now], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for &id in &ids {
        give_up(tx, kinds, id)?;
    }
    Ok(ids.len())
}

/// A due row as the worker attempts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueRow {
    pub id: i64,
    pub kind: String,
    pub address: String,
    pub subject: String,
    pub body: String,
    pub priority: String,
    pub attempts: u32,
}

/// What the worker does next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    Due(DueRow),
    /// Nothing due; the earliest next attempt, if any row waits.
    Idle(Option<i64>),
}

/// Gives up expired rows, deletes due placeholder rows unsent, then returns the due
/// row to attempt: security first, then next attempt time, then queue time.
pub fn pick(tx: &Transaction<'_>, kinds: &[&'static MailKind], now: i64) -> rusqlite::Result<Pick> {
    give_up_expired(tx, kinds, now)?;
    tx.execute(
        "DELETE FROM outbox WHERE placeholder = 1 AND outcome IS NULL AND next_attempt_at <= ?1",
        [now],
    )?;
    let due = tx
        .query_row(
            "SELECT o.id, o.kind, coalesce(o.address, u.email), o.subject, o.body,
                 o.priority, o.attempts
             FROM outbox o LEFT JOIN users u ON u.id = o.user_id
             WHERE o.outcome IS NULL AND o.next_attempt_at <= ?1
             ORDER BY o.priority = 'security' DESC, o.next_attempt_at, o.queued_at, o.id
             LIMIT 1",
            [now],
            |row| {
                Ok(DueRow {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    address: row.get(2)?,
                    subject: row.get(3)?,
                    body: row.get(4)?,
                    priority: row.get(5)?,
                    attempts: row.get(6)?,
                })
            },
        )
        .optional()?;
    match due {
        Some(row) => Ok(Pick::Due(row)),
        None => tx
            .query_row(
                "SELECT min(next_attempt_at) FROM outbox WHERE outcome IS NULL",
                [],
                |row| row.get(0),
            )
            .map(Pick::Idle),
    }
}

/// How an attempt ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Sent,
    Failed(Failure),
}

/// Records an attempt's outcome by row id; a row deleted meanwhile stays deleted.
pub fn record(
    tx: &Transaction<'_>,
    kinds: &[&'static MailKind],
    id: i64,
    outcome: Outcome,
    ended_at: i64,
) -> rusqlite::Result<()> {
    let row: Option<(bool, u32, i64, i64)> = tx
        .query_row(
            "SELECT token, attempts, queued_at, expires_at FROM outbox
             WHERE id = ?1 AND outcome IS NULL",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((token, attempts, queued_at, expires_at)) = row else {
        return Ok(());
    };
    let attempts = attempts + 1;
    match outcome {
        Outcome::Sent if token => {
            tx.execute("DELETE FROM outbox WHERE id = ?1", [id])?;
        }
        Outcome::Sent => {
            tx.execute(
                "UPDATE outbox SET outcome = 'sent', attempts = ?2 WHERE id = ?1",
                params![id, attempts],
            )?;
        }
        Outcome::Failed(_) => {
            match next_attempt(attempts, ended_at, deadline(queued_at, expires_at)) {
                Some(next) => {
                    tx.execute(
                        "UPDATE outbox SET attempts = ?2, next_attempt_at = ?3 WHERE id = ?1",
                        params![id, attempts, next],
                    )?;
                }
                None => {
                    tx.execute(
                        "UPDATE outbox SET attempts = ?2 WHERE id = ?1",
                        params![id, attempts],
                    )?;
                    give_up(tx, kinds, id)?;
                }
            }
        }
    }
    Ok(())
}

/// The one background worker delivering the outbox.
pub struct Worker {
    pub db: Arc<Db>,
    pub mailer: Arc<dyn Mailer>,
    pub kinds: &'static [&'static MailKind],
    pub wakeup: Wakeup,
    pub sender: SenderAddress,
    /// [`ATTEMPT_TIMEOUT`]; shorter in tests.
    pub attempt_timeout: Duration,
}

impl Worker {
    /// Runs until `stop` changes; an attempt in flight then is abandoned unrecorded, so
    /// the row is retried after the next start.
    pub async fn run(self, mut stop: watch::Receiver<()>) {
        loop {
            let kinds = self.kinds;
            let picked = self
                .db
                .write(move |tx| pick(tx, kinds, now_unix()).map_err(DbFailure::from))
                .await;
            let wait = match picked {
                Ok(Pick::Due(row)) => {
                    tokio::select! {
                        () = self.attempt(row) => continue,
                        _ = stop.changed() => return,
                    }
                }
                Ok(Pick::Idle(next)) => {
                    next.map(|at| Duration::from_secs(u64::try_from(at - now_unix()).unwrap_or(0)))
                }
                Err(failure) => {
                    tracing::error!("outbox worker cannot read the outbox: {failure}");
                    Some(WORKER_ERROR_PAUSE)
                }
            };
            let sleep = async {
                match wait {
                    Some(duration) => tokio::time::sleep(duration).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                () = self.wakeup.0.notified() => {}
                () = sleep => {}
                _ = stop.changed() => return,
            }
        }
    }

    /// One delivery attempt in its own task under [`ATTEMPT_TIMEOUT`]: a panic or stall
    /// is one failure of this row only.
    async fn attempt(&self, row: DueRow) {
        let mailer = Arc::clone(&self.mailer);
        let sender = self.sender.clone();
        let (id, address, subject_text, body) = (row.id, row.address, row.subject, row.body);
        let task = tokio::spawn(async move {
            let message =
                build_message(&sender, &address, &subject_text, &body).map_err(|_| Failure {
                    class: FailureClass::Build,
                    code: None,
                })?;
            mailer
                .send(Outgoing {
                    row_id: id,
                    message,
                    body,
                })
                .await
        });
        let abort = task.abort_handle();
        let result = match tokio::time::timeout(self.attempt_timeout, task).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(Failure {
                class: FailureClass::Panic,
                code: None,
            }),
            Err(_) => {
                abort.abort();
                Err(Failure {
                    class: FailureClass::Timeout,
                    code: None,
                })
            }
        };
        let outcome = match result {
            Ok(()) => Outcome::Sent,
            Err(failure) => Outcome::Failed(failure),
        };
        let attempt = row.attempts + 1;
        match outcome {
            Outcome::Sent => tracing::info!(
                row = id,
                kind = row.kind,
                priority = row.priority,
                attempt,
                outcome = "sent"
            ),
            Outcome::Failed(failure) => tracing::warn!(
                row = id,
                kind = row.kind,
                priority = row.priority,
                attempt,
                outcome = "failed",
                class = failure.class.name(),
                code = failure.code
            ),
        }
        let kinds = self.kinds;
        let recorded = self
            .db
            .write(move |tx| record(tx, kinds, id, outcome, now_unix()).map_err(DbFailure::from))
            .await;
        if let Err(failure) = recorded {
            tracing::error!(row = id, "cannot record a delivery outcome: {failure}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_retry_timeline() {
        let minute = 60;
        for (expiry, expected) in [
            (48 * 60 * minute, {
                let mut at = vec![0, 1, 6, 36];
                at.extend((1..=11).map(|k| 36 + 120 * k));
                at
            }),
            (10 * minute, vec![0, 1, 6]),
        ] {
            let queued = 1_000_000;
            let end = deadline(queued, queued + expiry);
            let mut attempts = vec![queued];
            let mut failures = 0;
            while let Some(next) = next_attempt(
                {
                    failures += 1;
                    failures
                },
                *attempts.last().unwrap(),
                end,
            ) {
                attempts.push(next);
            }
            let minutes: Vec<i64> = attempts.iter().map(|t| (t - queued) / minute).collect();
            assert_eq!(minutes, expected);
        }
        assert_eq!(
            36 + 120 * 11,
            22 * 60 + 36,
            "the 15th attempt is at 22 h 36 min"
        );
    }
}
