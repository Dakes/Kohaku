//! The public mail budget (request-limits: Public mail budget).

use std::net::IpAddr;
use std::time::{Duration, Instant};

use super::rate::{Budget, Demand, Limiter, SingleBucket};

/// Mails per hour one source (IPv4 /32 or IPv6 /48) may cause.
const PER_SOURCE_PER_HOUR: u32 = 10;

const HOUR: Duration = Duration::from_secs(3600);

/// The per-source buckets (in the limiter, so they count toward its cap) and the one
/// instance-wide bucket.
pub struct MailBudget {
    instance: SingleBucket,
}

impl MailBudget {
    /// `per_hour` is `KOHAKU_PUBLIC_MAIL_PER_HOUR`.
    pub fn new(per_hour: u32) -> MailBudget {
        MailBudget {
            instance: SingleBucket::new(Budget::per(per_hour, HOUR)),
        }
    }

    /// Takes one token from the source's bucket and one from the instance-wide bucket,
    /// or none when either is empty. The caller answers `false` with 429 "try again
    /// later" and queues no mail.
    pub fn take(&self, limiter: &Limiter, address: IpAddr, now: Instant) -> bool {
        let demand = Demand::mail_source(address, Budget::per(PER_SOURCE_PER_HOUR, HOUR));
        limiter.admit_with(
            &demand,
            now,
            |t| self.instance.admits(t),
            |t| self.instance.take(t),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn attacker_from_one_network() {
        let start = Instant::now();
        for sources in [
            vec![ip("203.0.113.5"); 11],
            (1..=11u16)
                .map(|n| IpAddr::V6((0x2001_0db8_0001_u128 << 80 | u128::from(n) << 64 | 1).into()))
                .collect(),
        ] {
            let limiter = Limiter::new(start);
            let budget = MailBudget::new(60);
            let admitted: Vec<bool> = sources
                .iter()
                .map(|&a| budget.take(&limiter, a, start))
                .collect();
            assert_eq!(admitted.iter().filter(|&&a| a).count(), 10);
            assert!(!admitted[10]);
            assert_eq!(budget.instance.tokens(Duration::ZERO), 50);
        }
    }

    #[test]
    fn distributed_attacker_hits_the_instance_cap() {
        let start = Instant::now();
        let limiter = Limiter::new(start);
        let budget = MailBudget::new(60);
        let admitted = (0..61u64)
            .filter(|&n| {
                let address = IpAddr::V6(((0x2001_u128 << 112) | (u128::from(n) << 80)).into());
                budget.take(&limiter, address, start)
            })
            .count();
        assert_eq!(admitted, 60);
    }

    #[test]
    fn configured_budget_and_refill() {
        let start = Instant::now();
        let limiter = Limiter::new(start);
        let budget = MailBudget::new(120);
        let mut admitted = 0;
        for source in 1..=13u32 {
            for _ in 0..10 {
                if budget.take(&limiter, IpAddr::V4((0xc000_0200 + source).into()), start) {
                    admitted += 1;
                }
            }
        }
        assert_eq!(admitted, 120);
        let later = start + Duration::from_secs(30);
        let fresh = IpAddr::V4(0xc000_0263.into());
        let two = (0..2)
            .filter(|_| budget.take(&limiter, fresh, later))
            .count();
        assert_eq!(two, 1);
    }
}
