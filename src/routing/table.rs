//! Every route as data, with its security declarations (http-security: Per-route
//! security declarations; change foundation D12). `Route` has no `Default`, so a new
//! declaration fails to compile at every entry until each states it.

use axum::routing::{MethodRouter, get};

use super::AppState;
use crate::limits::rate::RateClass;

/// Which router serves a route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKind {
    /// Matched by exact path before host routing, for any `Host`.
    Internal,
    Main,
    Project,
    /// The main and every project router.
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutePath {
    /// An axum path pattern such as `/static/{name}`.
    Pattern(&'static str),
    /// The router's fallback: every path no pattern matches.
    Fallback,
}

/// Who may use a route. Later changes add the admin and project guards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Public,
}

/// `Cache-Control` of every response (http-security: Cache-Control classes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheClass {
    NoStore,
    NoCache,
    /// `immutable` only on a 200 GET or HEAD serving a content-hashed asset, else
    /// `no-cache`.
    StaticAsset,
}

/// The route's Content-Security-Policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Csp {
    /// The release policy the header layer sets.
    Release,
}

/// Body cap and request deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyClass {
    /// 64 KiB, 15 s.
    Default,
}

impl BodyClass {
    pub fn cap(self) -> usize {
        match self {
            BodyClass::Default => 64 * 1024,
        }
    }

    pub fn deadline(self) -> std::time::Duration {
        match self {
            BodyClass::Default => std::time::Duration::from_secs(15),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Multipart {
    Rejected,
}

/// Whether a request with neither `Origin` nor `Sec-Fetch-Site` may pass the origin
/// check (JSON API writes and one-click unsubscribe only, from later changes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Headerless {
    NotExempt,
    /// Passes rule 3 only; no route of this change declares it.
    Exempt,
}

/// Body format of error responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorFormat {
    Html,
    Json,
}

pub struct Route {
    pub host: HostKind,
    pub path: RoutePath,
    /// A concrete path the route matrix requests.
    pub example: String,
    pub methods: MethodRouter<AppState>,
    pub access: Access,
    pub cache: CacheClass,
    pub csp: Csp,
    pub body: BodyClass,
    pub multipart: Multipart,
    pub headerless: Headerless,
    /// Rate classes drawn from, each only for the methods it covers.
    pub rate: &'static [RateClass],
    pub errors: ErrorFormat,
}

/// Every route Kohaku serves.
pub fn table() -> Vec<Route> {
    let mut routes = vec![
        Route {
            host: HostKind::Internal,
            path: RoutePath::Pattern("/healthz"),
            example: "/healthz".to_owned(),
            methods: get(super::internal::healthz),
            access: Access::Public,
            cache: CacheClass::NoStore,
            csp: Csp::Release,
            body: BodyClass::Default,
            multipart: Multipart::Rejected,
            headerless: Headerless::NotExempt,
            rate: &[],
            errors: ErrorFormat::Json,
        },
        Route {
            host: HostKind::Internal,
            path: RoutePath::Pattern(super::internal::TLS_ASK),
            example: "/.well-known/kohaku/tls-ask?domain=bugs.example.net".to_owned(),
            methods: get(super::internal::tls_ask),
            access: Access::Public,
            cache: CacheClass::NoStore,
            csp: Csp::Release,
            body: BodyClass::Default,
            multipart: Multipart::Rejected,
            headerless: Headerless::NotExempt,
            rate: &[],
            errors: ErrorFormat::Json,
        },
        Route {
            host: HostKind::Main,
            path: RoutePath::Pattern("/"),
            example: "/".to_owned(),
            methods: get(crate::pages::landing),
            access: Access::Public,
            cache: CacheClass::NoCache,
            csp: Csp::Release,
            body: BodyClass::Default,
            multipart: Multipart::Rejected,
            headerless: Headerless::NotExempt,
            rate: &[RateClass::Read],
            errors: ErrorFormat::Html,
        },
        Route {
            host: HostKind::Both,
            path: RoutePath::Pattern("/static/{name}"),
            example: crate::assets::path("kohaku.css").to_owned(),
            methods: get(crate::assets::serve),
            access: Access::Public,
            cache: CacheClass::StaticAsset,
            csp: Csp::Release,
            body: BodyClass::Default,
            multipart: Multipart::Rejected,
            headerless: Headerless::NotExempt,
            rate: &[],
            errors: ErrorFormat::Html,
        },
        Route {
            host: HostKind::Main,
            path: RoutePath::Fallback,
            example: "/does-not-exist".to_owned(),
            methods: get(crate::pages::not_found).fallback(crate::pages::not_found),
            access: Access::Public,
            cache: CacheClass::NoStore,
            csp: Csp::Release,
            body: BodyClass::Default,
            multipart: Multipart::Rejected,
            headerless: Headerless::NotExempt,
            rate: &[RateClass::Read],
            errors: ErrorFormat::Html,
        },
        Route {
            host: HostKind::Project,
            path: RoutePath::Fallback,
            example: "/does-not-exist".to_owned(),
            methods: get(crate::pages::not_found).fallback(crate::pages::not_found),
            access: Access::Public,
            cache: CacheClass::NoStore,
            csp: Csp::Release,
            body: BodyClass::Default,
            multipart: Multipart::Rejected,
            headerless: Headerless::NotExempt,
            rate: &[RateClass::Read],
            errors: ErrorFormat::Html,
        },
    ];
    routes.extend(crate::dev::routes());
    routes
}
