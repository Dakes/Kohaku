//! Per-network token buckets (request-limits: Rate-limit keys by network prefix,
//! Bounded limiter memory; design §3 Rate limiter).
//!
//! Each bucket is kept as its theoretical arrival time (GCRA): a bucket of `capacity`
//! tokens regaining one per `interval` is full once that time is not after now, and
//! admits a request while it lies at most `capacity - 1` intervals ahead. Integer time,
//! so refill is exact, and callers pass the time, so tests drive it.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::http::Method;

/// Buckets held at most, every class, /48 aggregate and per-source mail bucket counted.
pub const MAX_BUCKETS: usize = 100_000;

/// A rate-limit class of routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RateClass {
    /// Every GET and HEAD to a page or API route.
    Read,
    /// Every POST checking a password, a second factor or a single-use account token.
    Login,
    /// `POST /admin/reset`.
    Reset,
}

impl RateClass {
    pub const ALL: &'static [RateClass] = &[RateClass::Read, RateClass::Login, RateClass::Reset];

    /// Name in the per-minute rejection counters.
    pub fn name(self) -> &'static str {
        match self {
            RateClass::Read => "read",
            RateClass::Login => "login",
            RateClass::Reset => "reset",
        }
    }

    /// Requests per window for one IPv4 /32 or IPv6 /64; a /48 gets 8 times this.
    fn budget(self) -> Budget {
        match self {
            RateClass::Read => Budget::per(300, Duration::from_secs(60)),
            RateClass::Login => Budget::per(10, Duration::from_secs(15 * 60)),
            RateClass::Reset => Budget::per(5, Duration::from_secs(60 * 60)),
        }
    }

    /// Whether a request with `method` draws from this class.
    pub fn applies_to(self, method: &Method) -> bool {
        match self {
            RateClass::Read => method == Method::GET || method == Method::HEAD,
            RateClass::Login | RateClass::Reset => method == Method::POST,
        }
    }
}

/// `capacity` tokens, one regained every `interval`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    capacity: u32,
    interval: Duration,
}

impl Budget {
    pub fn per(capacity: u32, window: Duration) -> Budget {
        Budget {
            capacity,
            interval: window / capacity,
        }
    }

    fn times(self, factor: u32) -> Budget {
        Budget {
            capacity: self.capacity * factor,
            interval: self.interval / factor,
        }
    }
}

/// IPv6 /48 aggregates hold this many times the per-/64 budget.
const AGGREGATE_FACTOR: u32 = 8;

/// The network a bucket counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Prefix {
    V4(u32),
    V6Slash64(u64),
    V6Slash48(u64),
}

/// The /32 or /64 of a (canonical) client address, plus its /48 for IPv6.
pub fn prefixes(address: IpAddr) -> (Prefix, Option<Prefix>) {
    match address {
        IpAddr::V4(v4) => (Prefix::V4(u32::from(v4)), None),
        IpAddr::V6(v6) => {
            let bits = u128::from(v6);
            let top = u64::try_from(bits >> 64).expect("128 - 64 bits fit");
            (Prefix::V6Slash64(top), Some(Prefix::V6Slash48(top >> 16)))
        }
    }
}

/// The source of a mail bucket: IPv4 /32 or IPv6 /48.
pub fn mail_source(address: IpAddr) -> Prefix {
    match prefixes(address) {
        (v4, None) => v4,
        (_, Some(slash48)) => slash48,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Class(RateClass, Prefix),
    MailSource(Prefix),
    /// A per-address mail bucket: the address's HMAC under a per-boot key.
    MailAddress([u8; 32]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Overflow {
    Class(RateClass),
    MailSource,
    MailAddress,
}

/// A bucket: its budget and theoretical arrival time since the limiter's start.
#[derive(Debug, Clone, Copy)]
struct Bucket {
    budget: Budget,
    tat: Duration,
}

impl Bucket {
    fn full(budget: Budget, now: Duration) -> Bucket {
        Bucket { budget, tat: now }
    }

    fn admits(&self, now: Duration) -> bool {
        let tolerance = self.budget.interval * (self.budget.capacity - 1);
        self.tat <= now + tolerance
    }

    fn take(&mut self, now: Duration) {
        self.tat = self.tat.max(now) + self.budget.interval;
    }

    fn is_full(&self, now: Duration) -> bool {
        self.tat <= now
    }
}

/// One request's demand on the limiter: buckets by key with their budgets, and the
/// overflow bucket used instead when a missing one cannot be created.
pub struct Demand {
    keys: Vec<(Key, Budget)>,
    overflow: (Overflow, Budget),
}

impl Demand {
    pub fn class(class: RateClass, address: IpAddr) -> Demand {
        let budget = class.budget();
        let (own, aggregate) = prefixes(address);
        let mut keys = vec![(Key::Class(class, own), budget)];
        if let Some(slash48) = aggregate {
            keys.push((Key::Class(class, slash48), budget.times(AGGREGATE_FACTOR)));
        }
        Demand {
            keys,
            overflow: (Overflow::Class(class), budget),
        }
    }

    pub fn mail_address(key: [u8; 32], budget: Budget) -> Demand {
        Demand {
            keys: vec![(Key::MailAddress(key), budget)],
            overflow: (Overflow::MailAddress, budget),
        }
    }

    pub fn mail_source(address: IpAddr, budget: Budget) -> Demand {
        Demand {
            keys: vec![(Key::MailSource(mail_source(address)), budget)],
            overflow: (Overflow::MailSource, budget),
        }
    }
}

struct Inner {
    buckets: HashMap<Key, Bucket>,
    overflow: HashMap<Overflow, Bucket>,
}

/// Every bucket, in memory only, behind one lock.
pub struct Limiter {
    start: Instant,
    inner: Mutex<Inner>,
}

impl Limiter {
    pub fn new(start: Instant) -> Limiter {
        Limiter {
            start,
            inner: Mutex::new(Inner {
                buckets: HashMap::new(),
                overflow: HashMap::new(),
            }),
        }
    }

    fn elapsed(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.start)
    }

    /// Admits the request only when every bucket it needs holds a token, then takes one
    /// from each; a rejected request takes none.
    pub fn admit(&self, demand: &Demand, now: Instant) -> bool {
        self.admit_with(demand, now, |_| true, |_| {})
    }

    /// [`Limiter::admit`] plus one more bucket (the instance-wide mail budget), checked
    /// and taken with the others under the same lock.
    pub(crate) fn admit_with(
        &self,
        demand: &Demand,
        now: Instant,
        extra_admits: impl FnOnce(Duration) -> bool,
        extra_take: impl FnOnce(Duration),
    ) -> bool {
        let now = self.elapsed(now);
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let missing = demand
            .keys
            .iter()
            .filter(|(key, _)| !inner.buckets.contains_key(key))
            .count();
        let use_overflow = missing > 0 && inner.buckets.len() + missing > MAX_BUCKETS;
        let (overflow_key, overflow_budget) = demand.overflow;
        let admitted = demand
            .keys
            .iter()
            .all(|(key, budget)| match inner.buckets.get(key) {
                Some(bucket) => bucket.admits(now),
                None if use_overflow => true,
                None => Bucket::full(*budget, now).admits(now),
            })
            && (!use_overflow
                || inner
                    .overflow
                    .get(&overflow_key)
                    .is_none_or(|bucket| bucket.admits(now)))
            && extra_admits(now);
        if !admitted {
            return false;
        }
        for (key, budget) in &demand.keys {
            match inner.buckets.get_mut(key) {
                Some(bucket) => bucket.take(now),
                None if use_overflow => {}
                None => {
                    let mut bucket = Bucket::full(*budget, now);
                    bucket.take(now);
                    inner.buckets.insert(*key, bucket);
                }
            }
        }
        if use_overflow {
            inner
                .overflow
                .entry(overflow_key)
                .or_insert_with(|| Bucket::full(overflow_budget, now))
                .take(now);
        }
        extra_take(now);
        true
    }

    /// Removes every bucket refilled to its full budget (run every 60 s).
    pub fn sweep(&self, now: Instant) {
        let now = self.elapsed(now);
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner.buckets.retain(|_, bucket| !bucket.is_full(now));
        inner.overflow.retain(|_, bucket| !bucket.is_full(now));
    }

    /// Buckets held, overflow buckets excluded.
    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .buckets
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A shared bucket outside the keyed map (the instance-wide mail budget), checked and
/// taken under the limiter's lock by [`Limiter::admit_with`].
#[derive(Debug)]
pub(crate) struct SingleBucket {
    bucket: Mutex<Bucket>,
}

impl SingleBucket {
    pub(crate) fn new(budget: Budget) -> SingleBucket {
        SingleBucket {
            bucket: Mutex::new(Bucket::full(budget, Duration::ZERO)),
        }
    }

    pub(crate) fn admits(&self, now: Duration) -> bool {
        self.bucket
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .admits(now)
    }

    pub(crate) fn take(&self, now: Duration) {
        self.bucket
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take(now);
    }

    #[cfg(test)]
    pub(crate) fn tokens(&self, now: Duration) -> u32 {
        let bucket = *self.bucket.lock().unwrap();
        let ahead = bucket.tat.saturating_sub(now);
        let used = ahead.as_nanos().div_ceil(bucket.budget.interval.as_nanos());
        bucket.budget.capacity - u32::try_from(used).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        crate::limits::client_ip::canonical(text.parse().unwrap())
    }

    fn read(limiter: &Limiter, address: IpAddr, now: Instant) -> bool {
        limiter.admit(&Demand::class(RateClass::Read, address), now)
    }

    fn v6(slash64: u16, slash48: u16, host: u64) -> IpAddr {
        let bits = (0x2001_0db8_u128 << 96)
            | (u128::from(slash48) << 80)
            | (u128::from(slash64) << 64)
            | u128::from(host);
        IpAddr::V6(bits.into())
    }

    #[test]
    fn rotating_addresses_and_64s_inside_one_48() {
        let start = Instant::now();
        let limiter = Limiter::new(start);
        let first = (0..1300)
            .filter(|&h| read(&limiter, v6(1, 1, h + 1), start))
            .count();
        assert_eq!(first, 300);
        for net in 2..=8 {
            let admitted = (0..300)
                .filter(|&h| read(&limiter, v6(net, 1, h + 1), start))
                .count();
            assert_eq!(admitted, 300, "/64 number {net}");
        }
        assert!(!read(&limiter, v6(9, 1, 1), start));
        assert!(read(&limiter, ip("2001:db8:2::1"), start));
    }

    #[test]
    fn ipv4_attacker_cannot_exhaust_its_neighbours() {
        let start = Instant::now();
        let limiter = Limiter::new(start);
        for _ in 0..300 {
            assert!(read(&limiter, ip("::ffff:198.51.100.7"), start));
        }
        assert!(!read(&limiter, ip("198.51.100.7"), start));
        assert!(read(&limiter, ip("::ffff:198.51.100.8"), start));
    }

    #[test]
    fn refill_is_even_and_cannot_be_saved_up() {
        let start = Instant::now();
        let limiter = Limiter::new(start);
        let client = ip("198.51.100.7");
        while read(&limiter, client, start) {}
        let later = start + Duration::from_millis(200);
        assert!(read(&limiter, client, later));
        assert!(!read(&limiter, client, later));

        let attacker = ip("203.0.113.5");
        assert!(read(&limiter, attacker, start));
        let idle = start + Duration::from_secs(600);
        let admitted = (0..1000).filter(|_| read(&limiter, attacker, idle)).count();
        assert_eq!(admitted, 300);
    }

    #[test]
    fn slash48_regains_one_token_per_25_ms() {
        let start = Instant::now();
        let limiter = Limiter::new(start);
        // Empty the /48 through eight /64s, each emptied too.
        for net in 1..=8 {
            for h in 0..300 {
                assert!(read(&limiter, v6(net, 1, h + 1), start));
            }
        }
        let fresh = |t: Instant, n: u16| read(&limiter, v6(100 + n, 1, 1), t);
        assert!(!fresh(start, 0));
        let t = start + Duration::from_millis(25);
        assert!(fresh(t, 1));
        assert!(!fresh(t, 2));
    }

    #[test]
    fn flood_of_fresh_prefixes_stays_bounded() {
        let start = Instant::now();
        let limiter = Limiter::new(start);
        let client = ip("198.51.100.7");
        let mut admitted_without_own_bucket = 0;
        for n in 0..150_000u64 {
            let before = limiter.len();
            let address = IpAddr::V6(((0x2001_u128 << 112) | (u128::from(n) << 80)).into());
            let admitted = read(&limiter, address, start);
            if admitted && limiter.len() == before {
                admitted_without_own_bucket += 1;
            }
        }
        assert!(limiter.len() <= MAX_BUCKETS);
        assert_eq!(admitted_without_own_bucket, 300);
        // A client whose bucket existed before the flood keeps its own tokens.
        let limiter = Limiter::new(start);
        assert!(read(&limiter, client, start));
        for n in 0..150_000u64 {
            let address = IpAddr::V6(((0x2001_u128 << 112) | (u128::from(n) << 80)).into());
            read(&limiter, address, start);
        }
        assert!(read(&limiter, client, start));
    }

    #[test]
    fn sweep_cannot_reset_a_bucket() {
        let start = Instant::now();
        let limiter = Limiter::new(start);
        // Fill the map with idle buckets, then empty the overflow bucket.
        let mut n = 0u32;
        while limiter.len() < MAX_BUCKETS - 1 {
            read(&limiter, IpAddr::V4(n.into()), start);
            n += 1;
        }
        let attacker = ip("203.0.113.5");
        let emptied = start + Duration::from_secs(110);
        while read(&limiter, attacker, emptied) {}
        let mut m = 0u32;
        while read(&limiter, IpAddr::V4((0x0a00_0000 + m).into()), emptied) {
            m += 1;
        }
        assert_eq!(m, 300, "overflow admits one budget");
        let sweep_time = emptied + Duration::from_secs(10);
        limiter.sweep(sweep_time);
        assert_eq!(limiter.len(), 1, "only the attacker's bucket survives");
        assert!(read(&limiter, ip("192.0.2.200"), sweep_time));
        let burst = (0..1000)
            .filter(|_| read(&limiter, attacker, sweep_time))
            .count();
        assert_eq!(burst, 50);
    }
}
