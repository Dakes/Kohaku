//! Assembles the service from the route table: the one caller of axum's registration
//! methods (clippy.toml; change foundation D11, D12).

#![expect(
    clippy::disallowed_methods,
    reason = "routing::build is the one module that registers routes"
)]

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::Router;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, MatchedPath, Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use tower::util::BoxCloneSyncService;
use tower::{Service, ServiceBuilder, ServiceExt};
use tower_http::classify::{ServerErrorsAsFailures, SharedClassifier};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use super::AppState;
use super::hosts::{HostEntry, request_host};
use super::table::{Access, CacheClass, Csp, ErrorFormat, HostKind, Route, RoutePath};
use super::urls::Urls;
use crate::http::fetch::{admin_isolation_allows, admin_isolation_rejection, origin_layer};
use crate::http::{ClientAddr, PeerAddr, RoutedHost, body, errors, headers};
use crate::limits::client_ip::resolve;
use crate::limits::rate::{Demand, RateClass};
use crate::logging::{Reason, Rejected};

/// Marks a 200 response serving a content-hashed asset: only it gets `immutable`.
#[derive(Debug, Clone, Copy)]
pub struct HashedAsset;

const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// The whole HTTP service: header layer over the dispatcher.
pub type Service_ = BoxCloneSyncService<Request, Response, Infallible>;

/// Builds the service for `table` over `app`.
pub fn build(table: Vec<Route>, app: &AppState) -> Service_ {
    let mut internal = Router::new();
    let mut internal_paths = Vec::new();
    let mut main = Router::new();
    let mut project = Router::new();
    for route in table {
        let host = route.host;
        let path = route.path;
        let methods = layered(route, app);
        match (host, path) {
            (HostKind::Internal, RoutePath::Pattern(pattern)) => {
                internal_paths.push(pattern);
                internal = internal.route(pattern, methods);
            }
            (HostKind::Internal, RoutePath::Fallback) => {
                panic!("internal endpoints are exact paths and have no fallback")
            }
            (kind, RoutePath::Pattern(pattern)) => {
                if matches!(kind, HostKind::Main | HostKind::Both) {
                    main = main.route(pattern, methods.clone());
                }
                if matches!(kind, HostKind::Project | HostKind::Both) {
                    project = project.route(pattern, methods);
                }
            }
            (kind, RoutePath::Fallback) => {
                if matches!(kind, HostKind::Main | HostKind::Both) {
                    main = main.fallback_service(methods.clone().with_state::<()>(Arc::clone(app)));
                }
                if matches!(kind, HostKind::Project | HostKind::Both) {
                    project = project.fallback_service(methods.with_state::<()>(Arc::clone(app)));
                }
            }
        }
    }
    let host_router = |router: Router<AppState>| -> Router {
        router
            .layer(from_fn_with_state(Arc::clone(app), client_address_layer))
            .layer(trace_layer())
            .with_state(Arc::clone(app))
    };
    let dispatcher = Dispatcher {
        app: Arc::clone(app),
        internal: internal.with_state(Arc::clone(app)),
        internal_paths: internal_paths.into(),
        main: host_router(main),
        project: host_router(project),
    };
    let service = ServiceBuilder::new()
        .layer(from_fn_with_state(Arc::clone(app), headers::layer))
        .service(dispatcher);
    BoxCloneSyncService::new(service)
}

/// A route's method router under its declared layers, outermost first: cache class,
/// error body, deadline, origin check, rate classes, multipart, body cap, access.
fn layered(route: Route, app: &AppState) -> axum::routing::MethodRouter<AppState> {
    let Route {
        host,
        methods,
        access,
        cache,
        csp,
        body: body_class,
        multipart,
        headerless,
        rate,
        errors: format,
        ..
    } = route;
    let cap = body_class.cap();
    // The guard runs inside the body cap, so the CSRF check reads a capped body.
    let methods = match access {
        Access::Public => methods,
        Access::Session => methods.layer::<_, Infallible>(from_fn_with_state(
            Arc::clone(app),
            crate::auth::session::layer,
        )),
    };
    let methods = methods
        .layer::<_, Infallible>(DefaultBodyLimit::max(cap))
        .layer::<_, Infallible>(RequestBodyLimitLayer::new(cap));
    if host == HostKind::Internal {
        // No rate class, origin check or deadline before host routing (host-routing).
        return methods
            .layer::<_, Infallible>(from_fn_with_state(format, errors::layer))
            .layer::<_, Infallible>(from_fn_with_state((cache, csp), cache_layer));
    }
    methods
        .layer::<_, Infallible>(from_fn_with_state(multipart, body::multipart_layer))
        .layer::<_, Infallible>(from_fn_with_state((Arc::clone(app), rate), rate_layer))
        .layer::<_, Infallible>(from_fn_with_state(headerless, origin_layer))
        .layer::<_, Infallible>(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            body_class.deadline(),
        ))
        .layer::<_, Infallible>(from_fn_with_state(format, errors::layer))
        .layer::<_, Infallible>(from_fn_with_state((cache, csp), cache_layer))
}

/// Sets the declared `Cache-Control` and drops any CSP a handler set on a route
/// declaring the release policy.
async fn cache_layer(
    State((class, csp)): State<(CacheClass, Csp)>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let mut response = next.run(request).await;
    let value = match class {
        CacheClass::NoStore => "no-store",
        CacheClass::NoCache => "no-cache",
        CacheClass::StaticAsset => {
            let hashed = response.extensions().get::<HashedAsset>().is_some()
                && response.status() == StatusCode::OK
                && (method == Method::GET || method == Method::HEAD);
            if hashed { IMMUTABLE } else { "no-cache" }
        }
    };
    let headers = response.headers_mut();
    headers.remove(header::CACHE_CONTROL);
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(value));
    match csp {
        // The header layer then sets the release policy; a handler cannot weaken it.
        Csp::Release => {
            headers.remove(header::CONTENT_SECURITY_POLICY);
        }
    }
    response
}

/// Takes one token from every declared class covering the method, or answers 429.
async fn rate_layer(
    State((app, classes)): State<(AppState, &'static [RateClass])>,
    request: Request,
    next: Next,
) -> Response {
    let Some(ClientAddr(client)) = request.extensions().get::<ClientAddr>().copied() else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let now = app.clock.now();
    for &class in classes {
        if class.applies_to(request.method())
            && !app.limiter.admit(&Demand::class(class, client), now)
        {
            let mut response = StatusCode::TOO_MANY_REQUESTS.into_response();
            response
                .extensions_mut()
                .insert(Rejected(Reason::RateLimited(class)));
            return response;
        }
    }
    next.run(request).await
}

/// Resolves the client address of a host-routed request (request-limits).
async fn client_address_layer(
    State(app): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(PeerAddr(peer)) = request.extensions().get::<PeerAddr>().copied() else {
        tracing::error!("request without a peer address");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let resolution = resolve(
        peer.ip(),
        request.headers().get_all("x-forwarded-for"),
        &app.trusted_proxies,
    );
    for warning in resolution.warnings {
        app.warnings.note(warning);
    }
    request
        .extensions_mut()
        .insert(ClientAddr(resolution.client));
    next.run(request).await
}

type OnResponse = fn(&Response, std::time::Duration, &tracing::Span);
type Trace = TraceLayer<
    SharedClassifier<ServerErrorsAsFailures>,
    fn(&Request) -> tracing::Span,
    (),
    OnResponse,
    tower_http::trace::DefaultOnBodyChunk,
    tower_http::trace::DefaultOnEos,
    (),
>;

/// Request tracing with only the matched route pattern and the status (D21).
/// Rejections are only counted; a handler's 5xx is an error.
fn trace_layer() -> Trace {
    fn span(request: &Request) -> tracing::Span {
        match request.extensions().get::<MatchedPath>() {
            Some(pattern) => tracing::debug_span!("request", route = pattern.as_str()),
            None => tracing::debug_span!("request"),
        }
    }
    fn on_response(response: &Response, _: std::time::Duration, _: &tracing::Span) {
        if response.extensions().get::<Rejected>().is_some() {
            return;
        }
        let status = response.status().as_u16();
        if response.status().is_server_error() {
            tracing::error!(status, "request failed");
        } else {
            tracing::debug!(status, "request");
        }
    }
    TraceLayer::new(SharedClassifier::new(ServerErrorsAsFailures::new()))
        .make_span_with(span as fn(&Request) -> tracing::Span)
        .on_request(())
        .on_response(on_response as OnResponse)
        .on_failure(())
}

/// Chooses the router from the internal paths and the host map alone (D11, D15).
#[derive(Clone)]
struct Dispatcher {
    app: AppState,
    internal: Router,
    internal_paths: Arc<[&'static str]>,
    main: Router,
    project: Router,
}

type BoxFuture = Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send>>;

impl Service<Request> for Dispatcher {
    type Response = Response;
    type Error = Infallible;
    type Future = BoxFuture;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut request: Request) -> BoxFuture {
        if self.internal_paths.contains(&request.uri().path()) {
            return Box::pin(self.internal.clone().oneshot(request));
        }
        let entry = request_host(request.headers()).and_then(|host| {
            let map = self.app.host_map.current();
            let entry = map.get(&host)?;
            let origin = match entry {
                HostEntry::Main => self.app.urls.main_origin().to_owned(),
                HostEntry::Project(_) => Urls::project_origin(&host),
            };
            Some(RoutedHost { entry, origin })
        });
        let Some(routed) = entry else {
            let mut response = errors::response(StatusCode::MISDIRECTED_REQUEST, ErrorFormat::Html);
            response
                .extensions_mut()
                .insert(Rejected(Reason::UnknownHost));
            return Box::pin(std::future::ready(Ok(response)));
        };
        let router = match routed.entry {
            HostEntry::Main => {
                if !admin_isolation_allows(
                    request.method(),
                    request.uri().path(),
                    request.headers(),
                ) {
                    let (mut parts, _) = admin_isolation_rejection().into_parts();
                    let full = errors::response(StatusCode::FORBIDDEN, ErrorFormat::Html);
                    parts.headers = full.headers().clone();
                    let response = Response::from_parts(parts, full.into_body());
                    return Box::pin(std::future::ready(Ok(response)));
                }
                self.main.clone()
            }
            HostEntry::Project(project) => {
                request.extensions_mut().insert(project);
                self.project.clone()
            }
        };
        request.extensions_mut().insert(routed);
        Box::pin(router.oneshot(request))
    }
}

/// Converts a hyper request body into axum's, for the server.
pub fn into_axum_request<B>(request: axum::http::Request<B>) -> Request
where
    B: axum::body::HttpBody<Data = axum::body::Bytes> + Send + 'static,
    B::Error: Into<axum::BoxError>,
{
    request.map(Body::new)
}
