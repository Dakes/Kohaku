//! Concurrency bounds (request-limits: Bounded public write concurrency).

use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::logging::{Reason, Rejected};

/// Database writes for unauthenticated requests running at once.
pub const PUBLIC_WRITES: usize = 8;

/// A named concurrency bound; a full one answers 503 at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bound {
    PublicWrites,
}

impl Bound {
    pub const ALL: &'static [Bound] = &[Bound::PublicWrites];

    pub fn name(self) -> &'static str {
        match self {
            Bound::PublicWrites => "public_writes",
        }
    }
}

pub struct Permits {
    public_writes: Arc<Semaphore>,
}

impl Default for Permits {
    fn default() -> Permits {
        Permits {
            public_writes: Arc::new(Semaphore::new(PUBLIC_WRITES)),
        }
    }
}

impl Permits {
    /// A public write permit, released when dropped (success, error, deadline or
    /// disconnect alike); `None` when all are taken. Never waits.
    pub fn try_public_write(&self) -> Option<OwnedSemaphorePermit> {
        Arc::clone(&self.public_writes).try_acquire_owned().ok()
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
