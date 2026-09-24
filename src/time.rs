//! Wall-clock time as stored: INTEGER UTC unix seconds (change foundation D7).

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Whole seconds since the Unix epoch; 0 for a clock set before 1970.
pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// `unix` as `YYYY-MM-DD HH:MM UTC`, the time format of mail bodies.
pub fn format_utc_minute(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let seconds = unix.rem_euclid(86_400);
    // Howard Hinnant's days-to-civil algorithm, for the proleptic Gregorian calendar.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        seconds / 3600,
        seconds % 3600 / 60
    )
}

/// A manual clock's monotonic and wall time, moved together.
#[derive(Debug)]
pub struct ManualTime {
    instant: Instant,
    unix: i64,
}

/// Time for limiters, the scheduler and every expiry check: the system clock, or one
/// that tests set by hand (D30), so no test depends on how fast it runs.
#[derive(Debug, Clone)]
pub enum Clock {
    System,
    Manual(Arc<Mutex<ManualTime>>),
}

impl Clock {
    /// A manual clock stopped at the current instant and wall time.
    pub fn manual() -> Clock {
        Clock::Manual(Arc::new(Mutex::new(ManualTime {
            instant: Instant::now(),
            unix: now_unix(),
        })))
    }

    /// Monotonic time.
    pub fn now(&self) -> Instant {
        match self {
            Clock::System => Instant::now(),
            Clock::Manual(time) => time.lock().unwrap_or_else(PoisonError::into_inner).instant,
        }
    }

    /// Wall time in unix seconds, as stored.
    pub fn unix(&self) -> i64 {
        match self {
            Clock::System => now_unix(),
            Clock::Manual(time) => time.lock().unwrap_or_else(PoisonError::into_inner).unix,
        }
    }

    /// Moves a manual clock forward; a no-op on the system clock.
    pub fn advance(&self, by: Duration) {
        if let Clock::Manual(time) = self {
            let mut time = time.lock().unwrap_or_else(PoisonError::into_inner);
            time.instant += by;
            time.unix += i64::try_from(by.as_secs()).unwrap_or(i64::MAX);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_minutes() {
        assert_eq!(format_utc_minute(0), "1970-01-01 00:00 UTC");
        assert_eq!(format_utc_minute(951_782_400), "2000-02-29 00:00 UTC");
        assert_eq!(format_utc_minute(1_800_000_000), "2027-01-15 08:00 UTC");
        assert_eq!(format_utc_minute(4_107_542_399), "2100-02-28 23:59 UTC");
        assert_eq!(format_utc_minute(-1), "1969-12-31 23:59 UTC");
    }

    #[test]
    fn manual_clock_moves_both_times() {
        let clock = Clock::manual();
        let (instant, unix) = (clock.now(), clock.unix());
        clock.advance(Duration::from_secs(90));
        assert_eq!(clock.now() - instant, Duration::from_secs(90));
        assert_eq!(clock.unix() - unix, 90);
    }
}
