//! Everything the `dev` feature changes (distribution: Development live reload; change
//! foundation D23). A release build compiles only the no-op versions.

use axum::body::Body;

use crate::assets::Asset;
use crate::routing::table::Route;

/// The development CSP: the release policy plus exactly `connect-src 'self'`, which
/// the reload script's polling needs.
pub const DEV_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; \
    img-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'; \
    connect-src 'self'";

/// An asset's bytes: embedded in a release build.
#[cfg(not(feature = "dev"))]
pub fn asset_bytes(asset: &'static Asset) -> Result<Body, std::io::Error> {
    Ok(Body::from(asset.bytes))
}

/// An asset's bytes: read from `static/` on every request, by the registry's file name
/// (never a request path), so an edited stylesheet shows without a rebuild.
#[cfg(feature = "dev")]
pub fn asset_bytes(asset: &'static Asset) -> Result<Body, std::io::Error> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("static")
        .join(asset.name);
    std::fs::read(path).map(Body::from)
}

/// Routes only a `dev` build serves.
#[cfg(not(feature = "dev"))]
pub fn routes() -> Vec<Route> {
    Vec::new()
}

/// Routes only a `dev` build serves: the reload script and the boot id it polls, on
/// the main and every project router.
#[cfg(feature = "dev")]
pub fn routes() -> Vec<Route> {
    use axum::routing::get;

    use crate::limits::rate::RateClass;
    use crate::routing::table::{
        Access, BodyClass, CacheClass, Csp, ErrorFormat, Headerless, HostKind, Multipart, RoutePath,
    };
    vec![
        Route {
            host: HostKind::Both,
            path: RoutePath::Pattern(RELOAD_SCRIPT),
            example: RELOAD_SCRIPT.to_owned(),
            methods: get(reload_script),
            access: Access::Public,
            cache: CacheClass::NoStore,
            csp: Csp::Release,
            body: BodyClass::Default,
            multipart: Multipart::Rejected,
            headerless: Headerless::NotExempt,
            rate: &[],
            errors: ErrorFormat::Html,
        },
        Route {
            host: HostKind::Both,
            path: RoutePath::Pattern("/dev/boot-id"),
            example: "/dev/boot-id".to_owned(),
            methods: get(boot_id),
            access: Access::Public,
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

#[cfg(feature = "dev")]
const RELOAD_SCRIPT: &str = "/static/dev-reload.js";

#[cfg(feature = "dev")]
async fn reload_script() -> axum::response::Response {
    use axum::http::header;
    use axum::response::IntoResponse;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("static/dev-reload.js");
    match std::fs::read(path) {
        Ok(bytes) => ([(header::CONTENT_TYPE, "text/javascript")], bytes).into_response(),
        Err(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// A random id per process start: a new one tells the page to reload.
#[cfg(feature = "dev")]
async fn boot_id() -> String {
    use std::sync::OnceLock;
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        let mut bytes = [0u8; 16];
        crate::keys::random_bytes(&mut bytes).expect("the random source works in development");
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    })
    .clone()
}
