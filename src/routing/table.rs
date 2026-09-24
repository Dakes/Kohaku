//! Every route as data, with its security declarations (http-security: Per-route
//! security declarations; change foundation D12). `Route` has no `Default`, so a new
//! declaration fails to compile at every entry until each states it.

use axum::routing::{MethodRouter, any, get, post};

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

/// Who may use a route. Later changes add the project guards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Public,
    /// A signed-in account (admin-auth: Sessions): without a session GET and HEAD get
    /// `303` to the login page and other methods 403; with one, every state-changing
    /// request carries the session's CSRF token.
    Session,
    /// `Session`, then the admin only: any other account gets the 404 of a nonexistent
    /// path (change projects D2).
    Admin,
}

/// `Cache-Control` of every response (http-security: Cache-Control classes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheClass {
    NoStore,
    NoCache,
    /// `immutable` only on a 200 GET or HEAD serving a content-hashed asset, else
    /// `no-cache`.
    StaticAsset,
    /// `max-age=3600` on a 308 to a project's custom domain (projects: Canonical
    /// redirect to the custom domain), else `no-store`.
    CanonicalRedirect,
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
    routes.extend(admin_routes());
    routes.extend(project_routes());
    routes.extend(crate::dev::routes());
    routes
}

/// A reset token's shape, for the route matrix.
const EXAMPLE_TOKEN: &str = "/admin/reset/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

/// The admin area (admin-auth; change admin-auth D3): main host only, `no-store`.
fn admin_routes() -> Vec<Route> {
    use crate::admin::{account, login, projects, reset};
    vec![
        Route {
            host: HostKind::Main,
            path: RoutePath::Pattern("/admin"),
            example: "/admin".to_owned(),
            methods: get(crate::admin::home),
            access: Access::Session,
            cache: CacheClass::NoStore,
            csp: Csp::Release,
            body: BodyClass::Default,
            multipart: Multipart::Rejected,
            headerless: Headerless::NotExempt,
            rate: &[RateClass::Read],
            errors: ErrorFormat::Html,
        },
        Route {
            host: HostKind::Main,
            path: RoutePath::Pattern("/admin/login"),
            example: "/admin/login".to_owned(),
            methods: get(login::form).post(login::submit),
            access: Access::Public,
            cache: CacheClass::NoStore,
            csp: Csp::Release,
            body: BodyClass::Default,
            multipart: Multipart::Rejected,
            headerless: Headerless::NotExempt,
            rate: &[RateClass::Read, RateClass::Login],
            errors: ErrorFormat::Html,
        },
        Route {
            host: HostKind::Main,
            path: RoutePath::Pattern("/admin/logout"),
            example: "/admin/logout".to_owned(),
            methods: post(crate::admin::logout),
            access: Access::Session,
            cache: CacheClass::NoStore,
            csp: Csp::Release,
            body: BodyClass::Default,
            multipart: Multipart::Rejected,
            headerless: Headerless::NotExempt,
            rate: &[],
            errors: ErrorFormat::Html,
        },
        Route {
            host: HostKind::Main,
            path: RoutePath::Pattern("/admin/account"),
            example: "/admin/account".to_owned(),
            methods: get(account::show),
            access: Access::Session,
            cache: CacheClass::NoStore,
            csp: Csp::Release,
            body: BodyClass::Default,
            multipart: Multipart::Rejected,
            headerless: Headerless::NotExempt,
            rate: &[RateClass::Read],
            errors: ErrorFormat::Html,
        },
        Route {
            host: HostKind::Main,
            path: RoutePath::Pattern("/admin/account/password"),
            example: "/admin/account/password".to_owned(),
            methods: post(account::change_password),
            access: Access::Session,
            cache: CacheClass::NoStore,
            csp: Csp::Release,
            body: BodyClass::Default,
            multipart: Multipart::Rejected,
            headerless: Headerless::NotExempt,
            rate: &[RateClass::Login],
            errors: ErrorFormat::Html,
        },
        Route {
            host: HostKind::Main,
            path: RoutePath::Pattern("/admin/account/totp"),
            example: "/admin/account/totp".to_owned(),
            methods: post(account::start_totp),
            access: Access::Session,
            cache: CacheClass::NoStore,
            csp: Csp::Release,
            body: BodyClass::Default,
            multipart: Multipart::Rejected,
            headerless: Headerless::NotExempt,
            rate: &[RateClass::Login],
            errors: ErrorFormat::Html,
        },
        Route {
            host: HostKind::Main,
            path: RoutePath::Pattern("/admin/account/totp/confirm"),
            example: "/admin/account/totp/confirm".to_owned(),
            methods: post(account::confirm_totp),
            access: Access::Session,
            cache: CacheClass::NoStore,
            csp: Csp::Release,
            body: BodyClass::Default,
            multipart: Multipart::Rejected,
            headerless: Headerless::NotExempt,
            rate: &[RateClass::Login],
            errors: ErrorFormat::Html,
        },
        Route {
            host: HostKind::Main,
            path: RoutePath::Pattern("/admin/account/totp/disable"),
            example: "/admin/account/totp/disable".to_owned(),
            methods: post(account::disable_totp),
            access: Access::Session,
            cache: CacheClass::NoStore,
            csp: Csp::Release,
            body: BodyClass::Default,
            multipart: Multipart::Rejected,
            headerless: Headerless::NotExempt,
            rate: &[RateClass::Login],
            errors: ErrorFormat::Html,
        },
        Route {
            host: HostKind::Main,
            path: RoutePath::Pattern("/admin/account/recovery-codes"),
            example: "/admin/account/recovery-codes".to_owned(),
            methods: post(account::regenerate_codes),
            access: Access::Session,
            cache: CacheClass::NoStore,
            csp: Csp::Release,
            body: BodyClass::Default,
            multipart: Multipart::Rejected,
            headerless: Headerless::NotExempt,
            rate: &[RateClass::Login],
            errors: ErrorFormat::Html,
        },
        Route {
            host: HostKind::Main,
            path: RoutePath::Pattern("/admin/account/sessions/end-others"),
            example: "/admin/account/sessions/end-others".to_owned(),
            methods: post(account::end_other_sessions),
            access: Access::Session,
            cache: CacheClass::NoStore,
            csp: Csp::Release,
            body: BodyClass::Default,
            multipart: Multipart::Rejected,
            headerless: Headerless::NotExempt,
            rate: &[],
            errors: ErrorFormat::Html,
        },
        Route {
            host: HostKind::Main,
            path: RoutePath::Pattern("/admin/reset"),
            example: "/admin/reset".to_owned(),
            methods: get(reset::request_form).post(reset::request),
            access: Access::Public,
            cache: CacheClass::NoStore,
            csp: Csp::Release,
            body: BodyClass::Default,
            multipart: Multipart::Rejected,
            headerless: Headerless::NotExempt,
            rate: &[RateClass::Read, RateClass::Reset],
            errors: ErrorFormat::Html,
        },
        Route {
            host: HostKind::Main,
            path: RoutePath::Pattern("/admin/reset/{token}"),
            example: EXAMPLE_TOKEN.to_owned(),
            methods: get(reset::link_form).post(reset::link_submit),
            access: Access::Public,
            cache: CacheClass::NoStore,
            csp: Csp::Release,
            body: BodyClass::Default,
            multipart: Multipart::Rejected,
            headerless: Headerless::NotExempt,
            rate: &[RateClass::Read, RateClass::Login],
            errors: ErrorFormat::Html,
        },
        Route {
            host: HostKind::Main,
            path: RoutePath::Pattern("/admin/projects"),
            example: "/admin/projects".to_owned(),
            methods: get(projects::list).post(projects::create),
            access: Access::Admin,
            cache: CacheClass::NoStore,
            csp: Csp::Release,
            body: BodyClass::Default,
            multipart: Multipart::Rejected,
            headerless: Headerless::NotExempt,
            rate: &[RateClass::Read],
            errors: ErrorFormat::Html,
        },
        Route {
            host: HostKind::Main,
            path: RoutePath::Pattern("/admin/p/{slug}/settings"),
            example: "/admin/p/demo/settings".to_owned(),
            methods: get(projects::settings).post(projects::save),
            access: Access::Admin,
            cache: CacheClass::NoStore,
            csp: Csp::Release,
            body: BodyClass::Default,
            multipart: Multipart::Rejected,
            headerless: Headerless::NotExempt,
            rate: &[RateClass::Read],
            errors: ErrorFormat::Html,
        },
        Route {
            host: HostKind::Main,
            path: RoutePath::Pattern("/admin/p/{slug}/delete"),
            example: "/admin/p/demo/delete".to_owned(),
            methods: post(projects::delete),
            access: Access::Admin,
            cache: CacheClass::NoStore,
            csp: Csp::Release,
            body: BodyClass::Default,
            multipart: Multipart::Rejected,
            headerless: Headerless::NotExempt,
            rate: &[],
            errors: ErrorFormat::Html,
        },
        // Unknown admin paths answer like known ones until signed in, then 404. A
        // catch-all never matches an empty rest, so `/admin/` has its own entry.
        Route {
            host: HostKind::Main,
            path: RoutePath::Pattern("/admin/"),
            example: "/admin/".to_owned(),
            methods: any(crate::admin::not_found),
            access: Access::Session,
            cache: CacheClass::NoStore,
            csp: Csp::Release,
            body: BodyClass::Default,
            multipart: Multipart::Rejected,
            headerless: Headerless::NotExempt,
            rate: &[RateClass::Read],
            errors: ErrorFormat::Html,
        },
        Route {
            host: HostKind::Main,
            path: RoutePath::Pattern("/admin/{*rest}"),
            example: "/admin/does-not-exist".to_owned(),
            methods: any(crate::admin::not_found),
            access: Access::Session,
            cache: CacheClass::NoStore,
            csp: Csp::Release,
            body: BodyClass::Default,
            multipart: Multipart::Rejected,
            headerless: Headerless::NotExempt,
            rate: &[RateClass::Read],
            errors: ErrorFormat::Html,
        },
    ]
}

/// `/p/{slug}/…` on the main host: the canonical redirect (projects; change projects
/// D5). An empty rest does not match a catch-all, so `/p/{slug}/` has its own entry.
fn project_routes() -> Vec<Route> {
    ["/p/{slug}", "/p/{slug}/", "/p/{slug}/{*rest}"]
        .into_iter()
        .zip(["/p/demo", "/p/demo/", "/p/demo/r/12"])
        .map(|(pattern, example)| Route {
            host: HostKind::Main,
            path: RoutePath::Pattern(pattern),
            example: example.to_owned(),
            methods: get(crate::projects::redirect::canonical).fallback(crate::pages::not_found),
            access: Access::Public,
            cache: CacheClass::CanonicalRedirect,
            csp: Csp::Release,
            body: BodyClass::Default,
            multipart: Multipart::Rejected,
            headerless: Headerless::NotExempt,
            rate: &[RateClass::Read],
            errors: ErrorFormat::Html,
        })
        .collect()
}
