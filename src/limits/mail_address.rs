//! Per-address mail limit (request-limits: Per-address reset limit).

use std::time::{Duration, Instant};

use super::rate::{Budget, Demand, Limiter};
use crate::keys::{PerBootKey, Purpose, RandomSourceError};

/// Mails per hour one address may be sent through a public form; one regained every
/// 20 minutes.
const PER_ADDRESS_PER_HOUR: u32 = 3;

/// The buckets' keys: HMACs of normalized addresses under a key that lives only in
/// this process, so neither memory nor a log holds an address.
pub struct MailAddressLimit {
    key: PerBootKey,
}

impl MailAddressLimit {
    pub fn new() -> Result<MailAddressLimit, RandomSourceError> {
        Ok(MailAddressLimit {
            key: PerBootKey::generate()?,
        })
    }

    /// Takes one token from the bucket of `address` (a valid address), or none when it
    /// is empty.
    pub fn take(&self, limiter: &Limiter, address: &str, now: Instant) -> bool {
        let key = self
            .key
            .mac(Purpose::MailAddress, &[normalize(address).as_bytes()]);
        let budget = Budget::per(PER_ADDRESS_PER_HOUR, Duration::from_secs(60 * 60));
        limiter.admit(&Demand::mail_address(key, budget), now)
    }
}

/// Surrounding ASCII whitespace removed, ASCII-lowercased, any `+tag` removed from the
/// local part; no provider-specific rules.
pub fn normalize(address: &str) -> String {
    let lower = address
        .trim_matches(|c: char| c.is_ascii_whitespace())
        .to_ascii_lowercase();
    match lower.rsplit_once('@') {
        Some((local, domain)) => {
            let local = local.split_once('+').map_or(local, |(base, _)| base);
            format!("{local}@{domain}")
        }
        None => lower,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization() {
        for address in [
            "victim@example.org",
            "Victim+a@Example.org",
            " victim+b@example.org",
            "VICTIM@example.org\t",
            "victim+a+b@example.org",
        ] {
            assert_eq!(normalize(address), "victim@example.org", "{address}");
        }
        assert_eq!(normalize("v.ictim@example.org"), "v.ictim@example.org");
        assert_eq!(normalize("victim@ex+ample.org"), "victim@ex+ample.org");
    }

    #[test]
    fn three_per_hour_then_one_every_20_minutes() {
        let start = Instant::now();
        let limiter = Limiter::new(start);
        let limit = MailAddressLimit::new().unwrap();
        let taken: Vec<bool> = [
            "victim@example.org",
            "Victim+a@Example.org",
            " victim+b@example.org",
            "VICTIM@example.org",
        ]
        .iter()
        .map(|a| limit.take(&limiter, a, start))
        .collect();
        assert_eq!(taken, [true, true, true, false]);
        assert!(limit.take(&limiter, "other@example.org", start));
        let later = start + Duration::from_secs(20 * 60);
        assert!(limit.take(&limiter, "victim@example.org", later));
        assert!(!limit.take(&limiter, "victim@example.org", later));
    }
}
