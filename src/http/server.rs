//! The HTTP/1.1 server: header read timeout, peer address, graceful shutdown
//! (http-security: Request header read timeout; operations: Graceful shutdown; D10).
//! Not `axum::serve`, which has no header read timeout.

use std::convert::Infallible;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::{Bytes, HttpBody};
use axum::response::Response;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto::Builder;
use hyper_util::service::TowerToHyperService;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, watch};
use tokio::task::JoinSet;
use tower::Service;

use super::PeerAddr;
use crate::logging::Reason;
use crate::routing::AppState;
use crate::routing::build::{Service_, into_axum_request};

/// Time a request's line and headers may take to arrive.
pub const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Time requests in flight get after shutdown starts.
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(8);

/// How long shutdown lets a connection read bytes that arrived before it, so a request
/// being sent is not mistaken for an idle connection.
const ARRIVED_BYTES_WINDOW: Duration = Duration::from_millis(50);

/// Pause after a failed accept (such as running out of file descriptors).
const ACCEPT_ERROR_PAUSE: Duration = Duration::from_millis(100);

/// What shutdown needs to know about a connection: bytes read, requests in flight,
/// and the byte count when the last response was ready. More bytes than that with
/// nothing in flight means a request head is still arriving.
#[derive(Default)]
struct Activity {
    read: AtomicU64,
    in_flight: AtomicUsize,
    answered_at: AtomicU64,
    dispatched: Notify,
}

impl Activity {
    /// Whether closing now would cut off a request: one in flight, or a head arriving.
    fn busy(&self) -> bool {
        self.in_flight.load(Ordering::SeqCst) > 0
            || self.read.load(Ordering::SeqCst) > self.answered_at.load(Ordering::SeqCst)
    }

    /// Whether a request whose handling has started is in flight.
    fn handling(&self) -> bool {
        self.in_flight.load(Ordering::SeqCst) > 0
    }
}

/// The TCP stream, counting bytes read into its connection's [`Activity`].
struct Counted {
    stream: TcpStream,
    activity: Arc<Activity>,
}

impl AsyncRead for Counted {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.stream).poll_read(cx, buf);
        let read = u64::try_from(buf.filled().len() - before).unwrap_or(u64::MAX);
        self.activity.read.fetch_add(read, Ordering::SeqCst);
        result
    }
}

impl AsyncWrite for Counted {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

/// Passes each request to the service with the connection's peer address, tracking
/// requests in flight.
#[derive(Clone)]
struct WithPeer {
    service: Service_,
    peer: SocketAddr,
    activity: Arc<Activity>,
}

impl<B> Service<axum::http::Request<B>> for WithPeer
where
    B: HttpBody<Data = Bytes> + Send + 'static,
    B::Error: Into<axum::BoxError>,
{
    type Response = Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        self.service.poll_ready(cx)
    }

    fn call(&mut self, request: axum::http::Request<B>) -> Self::Future {
        let mut request = into_axum_request(request);
        request.extensions_mut().insert(PeerAddr(self.peer));
        let activity = Arc::clone(&self.activity);
        activity.in_flight.fetch_add(1, Ordering::SeqCst);
        activity.dispatched.notify_waiters();
        let response = self.service.call(request);
        Box::pin(async move {
            let response = response.await;
            activity
                .answered_at
                .store(activity.read.load(Ordering::SeqCst), Ordering::SeqCst);
            activity.in_flight.fetch_sub(1, Ordering::SeqCst);
            response
        })
    }
}

/// Serves `listener` until `shutdown` completes, then stops accepting, lets requests in
/// flight finish for at most [`SHUTDOWN_GRACE`] and closes every connection.
pub async fn run(
    listener: TcpListener,
    service: Service_,
    app: AppState,
    shutdown: impl Future<Output = ()>,
) {
    let mut builder = Builder::new(TokioExecutor::new()).http1_only();
    builder
        .http1()
        .timer(TokioTimer::new())
        .header_read_timeout(HEADER_READ_TIMEOUT);
    let (stop, stopped) = watch::channel(());
    let mut connections = JoinSet::new();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => {
                    let activity = Arc::new(Activity::default());
                    let service = WithPeer { service: service.clone(), peer, activity: Arc::clone(&activity) };
                    let io = Counted { stream, activity: Arc::clone(&activity) };
                    let connection = builder
                        .serve_connection(TokioIo::new(io), TowerToHyperService::new(service))
                        .into_owned();
                    let mut stopped = stopped.clone();
                    connections.spawn(async move {
                        tokio::pin!(connection);
                        // A connection error (a client gone, a timeout) concerns only it.
                        tokio::select! {
                            _ = connection.as_mut() => return,
                            _ = stopped.changed() => {}
                        }
                        // A head still arriving becomes a request first; the grace period
                        // (the caller's timeout) bounds the wait.
                        // hyper reads only when polled: let it take bytes that already arrived.
                        tokio::select! {
                            _ = connection.as_mut() => return,
                            () = tokio::time::sleep(ARRIVED_BYTES_WINDOW) => {}
                        }
                        while activity.busy() && !activity.handling() {
                            tokio::select! {
                                _ = connection.as_mut() => return,
                                () = activity.dispatched.notified() => {}
                                () = tokio::time::sleep(Duration::from_millis(50)) => {}
                            }
                        }
                        // Closes an idle connection now, a busy one after its response.
                        connection.as_mut().graceful_shutdown();
                        let _ = connection.await;
                    });
                }
                Err(_) => {
                    app.counters.increment(Reason::AcceptError);
                    tokio::time::sleep(ACCEPT_ERROR_PAUSE).await;
                }
            },
            () = &mut shutdown => break,
        }
        while connections.try_join_next().is_some() {}
    }
    drop(listener);
    let _ = stop.send(());
    let drained = async { while connections.join_next().await.is_some() {} };
    let _ = tokio::time::timeout(SHUTDOWN_GRACE, drained).await;
    connections.abort_all();
    while connections.join_next().await.is_some() {}
}
