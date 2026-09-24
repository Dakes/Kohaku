//! Log output and the per-minute rejection counters (operations: Log content privacy,
//! Rejected requests are logged as per-minute counters; change foundation D21).

use std::sync::atomic::{AtomicU64, Ordering};

use crate::limits::permits::Bound;
use crate::limits::rate::RateClass;

/// Why a request was rejected: a fixed set, never taken from request data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reason {
    UnknownHost,
    CrossSite,
    AdminIsolation,
    Deadline,
    BodyTooLarge,
    Multipart,
    RateLimited(RateClass),
    Busy(Bound),
    AcceptError,
}

impl Reason {
    /// Every reason, in the order they are logged.
    pub fn all() -> Vec<Reason> {
        let mut all = vec![
            Reason::UnknownHost,
            Reason::CrossSite,
            Reason::AdminIsolation,
            Reason::Deadline,
            Reason::BodyTooLarge,
            Reason::Multipart,
        ];
        all.extend(RateClass::ALL.iter().map(|&c| Reason::RateLimited(c)));
        all.extend(Bound::ALL.iter().map(|&b| Reason::Busy(b)));
        all.push(Reason::AcceptError);
        all
    }

    pub fn name(self) -> String {
        match self {
            Reason::UnknownHost => "unknown host (421)".to_owned(),
            Reason::CrossSite => "cross-site request (403)".to_owned(),
            Reason::AdminIsolation => "admin resource isolation (403)".to_owned(),
            Reason::Deadline => "request deadline exceeded (408)".to_owned(),
            Reason::BodyTooLarge => "body over its cap (413)".to_owned(),
            Reason::Multipart => "multipart not accepted (415)".to_owned(),
            Reason::RateLimited(class) => format!("rate limit {} (429)", class.name()),
            Reason::Busy(bound) => format!("concurrency bound {} full (503)", bound.name()),
            Reason::AcceptError => "connection accept error".to_owned(),
        }
    }
}

/// Marks a rejection response; the header layer counts it and nothing logs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rejected(pub Reason);

/// One counter per reason, logged and reset every 60 s.
pub struct Counters {
    reasons: Vec<Reason>,
    counts: Vec<AtomicU64>,
}

impl Default for Counters {
    fn default() -> Counters {
        let reasons = Reason::all();
        let counts = reasons.iter().map(|_| AtomicU64::new(0)).collect();
        Counters { reasons, counts }
    }
}

impl Counters {
    pub fn increment(&self, reason: Reason) {
        let index = self
            .reasons
            .iter()
            .position(|&r| r == reason)
            .expect("every reason has a counter");
        self.counts[index].fetch_add(1, Ordering::Relaxed);
    }

    /// The non-zero counts since the last call, which resets them.
    pub fn take(&self) -> Vec<(Reason, u64)> {
        self.reasons
            .iter()
            .zip(&self.counts)
            .map(|(&reason, count)| (reason, count.swap(0, Ordering::Relaxed)))
            .filter(|&(_, count)| count > 0)
            .collect()
    }

    /// Writes one INFO line per non-zero reason and resets the counts.
    pub fn flush(&self) {
        for (reason, count) in self.take() {
            tracing::info!("rejected requests: {}: {count}", reason.name());
        }
    }
}

/// Where a command's log goes: `serve` has no data output and logs to stdout; every
/// other command keeps stdout for data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// Installs the global subscriber: no ANSI, a fixed level (DEBUG in a `dev` build),
/// never read from the environment. Returns without effect when one is installed.
pub fn init(stream: Stream) {
    let level = if cfg!(feature = "dev") {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };
    let builder = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(level)
        .with_target(false);
    let _ = match stream {
        Stream::Stdout => builder.with_writer(std::io::stdout).try_init(),
        Stream::Stderr => builder.with_writer(std::io::stderr).try_init(),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_are_per_reason_and_restart_each_interval() {
        let counters = Counters::default();
        for _ in 0..3 {
            counters.increment(Reason::RateLimited(RateClass::Read));
        }
        for _ in 0..2 {
            counters.increment(Reason::CrossSite);
        }
        assert_eq!(
            counters.take(),
            [
                (Reason::CrossSite, 2),
                (Reason::RateLimited(RateClass::Read), 3)
            ]
        );
        counters.increment(Reason::RateLimited(RateClass::Read));
        assert_eq!(counters.take(), [(Reason::RateLimited(RateClass::Read), 1)]);
        assert!(counters.take().is_empty());
    }

    #[test]
    fn reason_names_are_unique() {
        let names: std::collections::HashSet<String> =
            Reason::all().into_iter().map(Reason::name).collect();
        assert_eq!(names.len(), Reason::all().len());
    }
}
