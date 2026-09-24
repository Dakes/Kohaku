//! Background work: one scheduler task for the limiter sweep, the counter flush and
//! retention (data-storage: Retention job, Secure deletion and WAL checkpoints; change
//! foundation D20).

use std::fs;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use tokio::sync::watch;

use rusqlite::params;

use crate::auth::session;
use crate::db::migrate::DbFailure;
use crate::db::{DataDir, Db};
use crate::mail::outbox::{MailKind, give_up_expired};
use crate::routing::AppState;
use crate::time::now_unix;

/// Limiter sweep and counter flush.
pub const SWEEP_EVERY: Duration = Duration::from_secs(60);

/// Retention (and its checkpoint), also run right after startup.
pub const RETENTION_EVERY: Duration = Duration::from_secs(60 * 60);

/// Audit entries are kept this long.
pub const AUDIT_KEEP_SECONDS: i64 = 365 * 24 * 60 * 60;

/// Temporary files older than this are left over from a crash.
pub const TMP_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Job {
    Sweep,
    Retention,
}

/// When each job is next due; pure, so tests drive it with any instants.
#[derive(Debug, Clone, Copy)]
pub struct Schedule {
    next_sweep: Instant,
    next_retention: Instant,
}

impl Schedule {
    /// Retention is due at once (after startup), the sweep a minute later.
    pub fn new(start: Instant) -> Schedule {
        Schedule {
            next_sweep: start + SWEEP_EVERY,
            next_retention: start,
        }
    }

    /// The jobs due at `now`, each rescheduled from its due time.
    pub fn due(&mut self, now: Instant) -> Vec<Job> {
        let mut jobs = Vec::new();
        if now >= self.next_sweep {
            jobs.push(Job::Sweep);
            while self.next_sweep <= now {
                self.next_sweep += SWEEP_EVERY;
            }
        }
        if now >= self.next_retention {
            jobs.push(Job::Retention);
            while self.next_retention <= now {
                self.next_retention += RETENTION_EVERY;
            }
        }
        jobs
    }

    pub fn next_wake(&self) -> Instant {
        self.next_sweep.min(self.next_retention)
    }
}

/// Runs the scheduler until `stop` changes; a job in progress finishes its current
/// transaction, and no new one starts.
pub async fn run(
    app: AppState,
    data: DataDir,
    kinds: &'static [&'static MailKind],
    mut stop: watch::Receiver<()>,
) {
    let mut schedule = Schedule::new(Instant::now());
    loop {
        for job in schedule.due(Instant::now()) {
            match job {
                Job::Sweep => {
                    app.limiter.sweep(app.clock.now());
                    app.counters.flush();
                }
                Job::Retention => {
                    tokio::select! {
                        () = retention(&app.db, &data, kinds, now_unix()) => {}
                        _ = stop.changed() => return,
                    }
                }
            }
        }
        let wake = tokio::time::Instant::from_std(schedule.next_wake());
        tokio::select! {
            () = tokio::time::sleep_until(wake) => {}
            _ = stop.changed() => return,
        }
    }
}

/// A retention step failed; it is retried at the next run.
fn step_failed(step: &str, failure: impl std::fmt::Display) {
    tracing::error!("retention step {step} failed: {failure}");
}

/// One retention run: each step its own transaction, a failure logged without row
/// content, then a truncating WAL checkpoint.
pub async fn retention(
    db: &Arc<Db>,
    data: &DataDir,
    kinds: &'static [&'static MailKind],
    now: i64,
) {
    let outbox = db
        .write(move |tx| {
            give_up_expired(tx, kinds, now)?;
            tx.execute("DELETE FROM outbox WHERE outcome IS NOT NULL", [])
                .map_err(DbFailure::from)
        })
        .await;
    if let Err(failure) = outbox {
        step_failed("outbox", failure);
    }
    let audit = db
        .write(move |tx| {
            tx.execute(
                "DELETE FROM audit_log WHERE time < ?1",
                [now - AUDIT_KEEP_SECONDS],
            )
            .map_err(DbFailure::from)
        })
        .await;
    if let Err(failure) = audit {
        step_failed("audit_log", failure);
    }
    let sign_in = db
        .write(move |tx| {
            tx.execute(
                "DELETE FROM sessions WHERE last_seen <= ?1 - ?2 OR created_at <= ?1 - ?3",
                params![now, session::IDLE_SECONDS, session::ABSOLUTE_SECONDS],
            )?;
            tx.execute(
                "DELETE FROM tokens
                 WHERE purpose = 'reset' AND (used_at IS NOT NULL OR expires_at <= ?1)",
                [now],
            )?;
            tx.execute(
                "DELETE FROM known_devices WHERE created_at <= ?1 - ?2",
                params![now, session::DEVICE_SECONDS],
            )
            .map_err(DbFailure::from)
        })
        .await;
    if let Err(failure) = sign_in {
        step_failed("sign-in state", failure);
    }
    let dirs = [data.root().to_path_buf(), data.backups()];
    let removed = tokio::task::spawn_blocking(move || {
        let now = SystemTime::now();
        dirs.iter().try_for_each(|dir| remove_stale_tmp(dir, now))
    })
    .await;
    match removed {
        Ok(Ok(())) => {}
        Ok(Err(error)) => step_failed("temporary files", error.kind()),
        Err(_) => step_failed("temporary files", "task failed"),
    }
    checkpoint(db).await;
}

/// `PRAGMA wal_checkpoint(TRUNCATE)`; a checkpoint blocked by a reader is retried at
/// the next run and fails no request.
pub async fn checkpoint(db: &Arc<Db>) -> bool {
    let result = db
        .with_writer(|conn| {
            conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                row.get::<_, i64>(0)
            })
        })
        .await;
    match result {
        Ok(0) => true,
        Ok(_) => {
            tracing::info!("WAL checkpoint blocked by a reader; retried at the next run");
            false
        }
        Err(error) => {
            step_failed("checkpoint", DbFailure::from(error));
            false
        }
    }
}

/// Removes regular `*.tmp` files directly in `dir` last modified before
/// `now - TMP_MAX_AGE`: no link followed, no subdirectory entered.
pub fn remove_stale_tmp(dir: &Path, now: SystemTime) -> io::Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        if !name.as_encoded_bytes().ends_with(b".tmp") {
            continue;
        }
        let path = entry.path();
        let meta = fs::symlink_metadata(&path)?;
        let stale = meta
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > TMP_MAX_AGE);
        if meta.file_type().is_file() && stale {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_after_startup_and_hourly_sweep_every_minute() {
        let start = Instant::now();
        let mut schedule = Schedule::new(start);
        assert_eq!(schedule.due(start), [Job::Retention]);
        assert_eq!(schedule.next_wake(), start + SWEEP_EVERY);
        let mut retentions = 1;
        let mut sweeps = 0;
        for minute in 1..=180u64 {
            let now = start + Duration::from_secs(minute * 60);
            for job in schedule.due(now) {
                match job {
                    Job::Sweep => sweeps += 1,
                    Job::Retention => retentions += 1,
                }
            }
        }
        assert_eq!(sweeps, 180);
        assert_eq!(retentions, 4, "at start, then at 60, 120 and 180 minutes");
        // A late wake runs each job once, not once per missed period.
        let late = start + Duration::from_secs(10 * 60 * 60);
        assert_eq!(schedule.due(late), [Job::Sweep, Job::Retention]);
        assert!(schedule.next_wake() > late);
    }
}
