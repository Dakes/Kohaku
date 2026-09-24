//! Concurrency bounds (request-limits: Bounded public write concurrency; admin-auth:
//! Password hashing is bounded).

use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::logging::{Reason, Rejected};

/// Database writes for unauthenticated requests running at once.
pub const PUBLIC_WRITES: usize = 8;

/// How long a password check waits for a hashing permit before 503.
pub const HASHING_WAIT: Duration = Duration::from_secs(2);

/// A named concurrency bound; a full one answers 503.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bound {
    PublicWrites,
    /// argon2id computations (19 MiB each).
    Hashing,
}

impl Bound {
    pub const ALL: &'static [Bound] = &[Bound::PublicWrites, Bound::Hashing];

    pub fn name(self) -> &'static str {
        match self {
            Bound::PublicWrites => "public_writes",
            Bound::Hashing => "hashing",
        }
    }
}

pub struct Permits {
    public_writes: Arc<Semaphore>,
    /// One argon2id permit for everyone.
    hashing: Arc<Semaphore>,
    /// One more, only for logins carrying a valid device cookie for their account.
    hashing_reserved: Arc<Semaphore>,
}

impl Default for Permits {
    fn default() -> Permits {
        Permits {
            public_writes: Arc::new(Semaphore::new(PUBLIC_WRITES)),
            hashing: Arc::new(Semaphore::new(1)),
            hashing_reserved: Arc::new(Semaphore::new(1)),
        }
    }
}

impl Permits {
    /// A public write permit, released when dropped (success, error, deadline or
    /// disconnect alike); `None` when all are taken. Never waits.
    pub fn try_public_write(&self) -> Option<OwnedSemaphorePermit> {
        Arc::clone(&self.public_writes).try_acquire_owned().ok()
    }

    /// A hashing permit within [`HASHING_WAIT`]: a request with a valid device cookie
    /// takes whichever of the two frees first, others only the general one. `None`
    /// on timeout.
    pub async fn hashing(&self, known_device: bool) -> Option<OwnedSemaphorePermit> {
        let general = Arc::clone(&self.hashing).acquire_owned();
        let permit = async {
            if known_device {
                let reserved = Arc::clone(&self.hashing_reserved).acquire_owned();
                tokio::select! {
                    biased;
                    permit = reserved => permit,
                    permit = general => permit,
                }
            } else {
                general.await
            }
        };
        match tokio::time::timeout(HASHING_WAIT, permit).await {
            Ok(Ok(permit)) => Some(permit),
            // The semaphores are never closed; a timeout is the only way to get here.
            Ok(Err(_)) | Err(_) => None,
        }
    }
}

/// The answer when `bound` is full: 503 at once, counted, never logged per request.
pub fn busy(bound: Bound) -> Response {
    let mut response = StatusCode::SERVICE_UNAVAILABLE.into_response();
    response
        .extensions_mut()
        .insert(Rejected(Reason::Busy(bound)));
    response
}
