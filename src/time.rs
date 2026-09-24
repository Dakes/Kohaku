//! Wall-clock time as stored: INTEGER UTC unix seconds (change foundation D7).

use std::time::{SystemTime, UNIX_EPOCH};

/// Whole seconds since the Unix epoch; 0 for a clock set before 1970.
pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Monotonic time for limiters and the scheduler: the system clock, or one that tests
/// set by hand (D30), so no test depends on how fast it runs.
#[derive(Debug, Clone)]
pub enum Clock {
    System,
    Manual(std::sync::Arc<std::sync::Mutex<std::time::Instant>>),
}

impl Clock {
    /// A manual clock stopped at the current instant.
    pub fn manual() -> Clock {
        Clock::Manual(std::sync::Arc::new(std::sync::Mutex::new(
            std::time::Instant::now(),
        )))
    }

    pub fn now(&self) -> std::time::Instant {
        match self {
            Clock::System => std::time::Instant::now(),
            Clock::Manual(now) => *now
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        }
    }

    /// Moves a manual clock forward; a no-op on the system clock.
    pub fn advance(&self, by: std::time::Duration) {
        if let Clock::Manual(now) = self {
            *now.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) += by;
        }
    }
}
