//! `kohaku serve` as a library entry point, with its listener, mailer and shutdown
//! injected (operations: Server start, Graceful shutdown; change foundation D10, D30).

use std::fmt;
use std::future::Future;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener};
use std::sync::Arc;

use tokio::sync::watch;

use crate::config::{MailTransport, ServeConfig};
use crate::db::lock::{InstanceLock, LockError};
use crate::db::migrate::{DbFailure, MIGRATIONS, MigrateError, open_and_prepare};
use crate::db::{DataDir, Db};
use crate::keys::RandomSourceError;
use crate::mail::Mailer;
use crate::mail::outbox::{ATTEMPT_TIMEOUT, KINDS, Worker};
use crate::mail::smtp::{SmtpMailer, SmtpRoots, SmtpSetupError};
use crate::routing::App;
use crate::routing::build::build;
use crate::routing::table::table;
use crate::time::{Clock, now_unix};

/// Linux's `EAFNOSUPPORT`: the host has no IPv6.
const EAFNOSUPPORT: i32 = 97;

/// Where to listen once startup has finished.
#[derive(Debug, Clone, Copy)]
pub enum Listen {
    /// `[::]:port`, dual-stack; `0.0.0.0:port` only on a host without IPv6.
    AllAddresses(u16),
    /// One address (tests).
    Address(SocketAddr),
}

/// Which mailer the outbox worker uses.
pub enum MailerChoice {
    /// From the configuration: SMTP over the compiled roots, or the `dev` printer.
    Configured,
    /// Injected (tests).
    Given(Arc<dyn Mailer>),
}

#[derive(Debug)]
pub enum ServeError {
    Lock(LockError),
    Database(MigrateError),
    Mail(SmtpSetupError),
    Bind(io::Error),
    Random(RandomSourceError),
}

impl fmt::Display for ServeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServeError::Lock(error) => error.fmt(f),
            ServeError::Database(error) => error.fmt(f),
            ServeError::Mail(error) => error.fmt(f),
            ServeError::Bind(error) => write!(f, "cannot listen on port 8080: {}", error.kind()),
            ServeError::Random(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ServeError {}

fn bind(listen: Listen) -> io::Result<TcpListener> {
    let listener = match listen {
        Listen::Address(address) => TcpListener::bind(address)?,
        Listen::AllAddresses(port) => {
            match TcpListener::bind(SocketAddr::from((Ipv6Addr::UNSPECIFIED, port))) {
                Err(error) if error.raw_os_error() == Some(EAFNOSUPPORT) => {
                    TcpListener::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)))?
                }
                other => other?,
            }
        }
    };
    listener.set_nonblocking(true)?;
    Ok(listener)
}

fn configured_mailer(config: &ServeConfig) -> Result<Arc<dyn Mailer>, ServeError> {
    match &config.transport {
        MailTransport::Smtp(smtp) => {
            let mailer = SmtpMailer::new(smtp, config.base_url.host(), SmtpRoots::compiled())
                .map_err(ServeError::Mail)?;
            Ok(Arc::new(mailer))
        }
        #[cfg(feature = "dev")]
        MailTransport::Print => Ok(Arc::new(crate::mail::dev::PrintMailer)),
    }
}

/// Starts the server and runs it until `shutdown` completes. Nothing listens before
/// the lock, keycheck and migrations have passed; `on_listening` gets the address.
pub async fn serve(
    config: ServeConfig,
    data: DataDir,
    listen: Listen,
    mailer: MailerChoice,
    on_listening: impl FnOnce(SocketAddr),
    shutdown: impl Future<Output = ()>,
) -> Result<(), ServeError> {
    let lock = InstanceLock::acquire(&data).map_err(ServeError::Lock)?;
    let (writer, data) = tokio::task::spawn_blocking(move || {
        let writer = open_and_prepare(&data, &config.secret, MIGRATIONS, now_unix());
        (writer.map(|w| (w, config)), data)
    })
    .await
    .expect("startup does not panic");
    let (writer, config) = writer.map_err(ServeError::Database)?;
    let db = Db::new(writer, &data.database())
        .map_err(|error| ServeError::Database(MigrateError::Database(DbFailure::from(error))))?;
    let mailer = match mailer {
        MailerChoice::Configured => configured_mailer(&config)?,
        MailerChoice::Given(mailer) => mailer,
    };
    let app = Arc::new(App::new(&config, Arc::new(db), Clock::System).map_err(ServeError::Random)?);
    // Made before listening, so the first failed login costs what every later one does.
    let dummy = tokio::task::spawn_blocking(crate::auth::password::dummy_hash)
        .await
        .expect("hashing does not panic");
    dummy.map_err(ServeError::Random)?;
    crate::routing::hosts::refresh(&app)
        .await
        .map_err(|failure| ServeError::Database(MigrateError::Database(failure)))?;
    tracing::info!(
        "KOHAKU_PUBLIC_MAIL_PER_HOUR is {}",
        config.public_mail_per_hour
    );
    let service = build(table(), &app);
    let listener = bind(listen).map_err(ServeError::Bind)?;
    let address = listener.local_addr().map_err(ServeError::Bind)?;
    let listener = tokio::net::TcpListener::from_std(listener).map_err(ServeError::Bind)?;

    let (stop, stopped) = watch::channel(());
    let worker = tokio::spawn(
        Worker {
            db: Arc::clone(&app.db),
            mailer,
            kinds: KINDS,
            wakeup: app.outbox.clone(),
            sender: config.sender.clone(),
            attempt_timeout: ATTEMPT_TIMEOUT,
        }
        .run(stopped.clone()),
    );
    let jobs = tokio::spawn(crate::jobs::run(
        Arc::clone(&app),
        data.clone(),
        KINDS,
        stopped.clone(),
    ));
    let watcher = tokio::spawn(crate::routing::hosts::watch(Arc::clone(&app), stopped));
    tracing::info!("listening on port {}", address.port());
    on_listening(address);

    // Background work stops as soon as shutdown starts, within the same grace period.
    let shutdown = async {
        shutdown.await;
        let _ = stop.send(());
    };
    crate::http::server::run(listener, service, Arc::clone(&app), shutdown).await;
    let _ = worker.await;
    let _ = jobs.await;
    let _ = watcher.await;
    app.counters.flush();
    // The last handle closes every connection; SQLite then removes the WAL.
    drop(app);
    drop(lock);
    tracing::info!("stopped");
    Ok(())
}
