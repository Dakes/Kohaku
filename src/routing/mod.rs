//! Routes, host routing and the state every handler shares (host-routing; change
//! foundation D12, D15).

pub mod build;
pub mod hosts;
pub mod internal;
pub mod table;
pub mod urls;

use std::sync::Arc;

use crate::config::{BaseUrl, ServeConfig, TrustedProxies};
use crate::db::Db;
use crate::keys::{InstanceSecret, RandomSourceError};
use crate::limits::client_ip::ClientWarnings;
use crate::limits::mail_address::MailAddressLimit;
use crate::limits::mail_budget::MailBudget;
use crate::limits::permits::Permits;
use crate::limits::rate::Limiter;
use crate::logging::Counters;
use crate::mail::outbox::Wakeup;
use crate::time::Clock;
use hosts::{HostMap, SharedHostMap};
use urls::Urls;

/// What every handler and layer shares.
pub struct App {
    pub base_url: BaseUrl,
    pub trusted_proxies: TrustedProxies,
    pub host_map: Arc<SharedHostMap>,
    pub urls: Urls,
    pub limiter: Limiter,
    pub mail_budget: MailBudget,
    pub mail_address: MailAddressLimit,
    pub permits: Permits,
    pub counters: Counters,
    pub warnings: ClientWarnings,
    pub db: Arc<Db>,
    pub outbox: Wakeup,
    pub clock: Clock,
    /// Derives TOTP seeds.
    pub secret: Arc<InstanceSecret>,
    /// Password checks run so far: tests see that every login hashes exactly once.
    pub password_checks: std::sync::atomic::AtomicU64,
}

pub type AppState = Arc<App>;

impl App {
    pub fn new(config: &ServeConfig, db: Arc<Db>, clock: Clock) -> Result<App, RandomSourceError> {
        Ok(App {
            base_url: config.base_url.clone(),
            trusted_proxies: config.trusted_proxies.clone(),
            host_map: Arc::new(SharedHostMap::new(HostMap::new(&config.base_url))),
            urls: Urls::new(&config.base_url),
            limiter: Limiter::new(clock.now()),
            mail_budget: MailBudget::new(config.public_mail_per_hour),
            mail_address: MailAddressLimit::new()?,
            permits: Permits::default(),
            counters: Counters::default(),
            warnings: ClientWarnings::default(),
            db,
            outbox: Wakeup::default(),
            clock,
            secret: Arc::new(config.secret.clone()),
            password_checks: std::sync::atomic::AtomicU64::new(0),
        })
    }
}
