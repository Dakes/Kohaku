//! The outermost layer: fixed security headers, CSP and Cache-Control when unset, and
//! the rejection counters (http-security: Security headers on every response, Exact
//! Content Security Policy; change foundation D13, D21).

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;

use crate::logging::{Reason, Rejected};
use crate::routing::AppState;

/// The release Content-Security-Policy.
pub const RELEASE_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; \
    img-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'";

/// The policy this build sends: a `dev` build adds exactly `connect-src 'self'`.
pub const CSP: &str = if cfg!(feature = "dev") {
    crate::dev::DEV_CSP
} else {
    RELEASE_CSP
};

/// Sent exactly once with exactly this value on every response, replacing any value
/// a handler set.
pub fn fixed_headers() -> [(HeaderName, &'static str); 6] {
    [
        (header::REFERRER_POLICY, "same-origin"),
        (header::X_FRAME_OPTIONS, "DENY"),
        (
            HeaderName::from_static("cross-origin-opener-policy"),
            "same-origin",
        ),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (HeaderName::from_static("permissions-policy"), ""),
        (header::STRICT_TRANSPORT_SECURITY, "max-age=31536000"),
    ]
}

/// Applies the header policy to a response headed out.
pub fn apply(headers: &mut HeaderMap) {
    for (name, value) in fixed_headers() {
        headers.remove(&name);
        headers.insert(name, HeaderValue::from_static(value));
    }
    set_single_if_absent(headers, header::CONTENT_SECURITY_POLICY, CSP);
    set_single_if_absent(headers, header::CACHE_CONTROL, "no-store");
}

fn set_single_if_absent(headers: &mut HeaderMap, name: HeaderName, value: &'static str) {
    if headers.get_all(&name).iter().count() != 1 {
        headers.remove(&name);
        headers.insert(name, HeaderValue::from_static(value));
    }
}

/// Outermost layer over the dispatcher.
pub async fn layer(State(app): State<AppState>, request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let reason = match response.extensions().get::<Rejected>() {
        Some(Rejected(reason)) => Some(*reason),
        None => match response.status() {
            StatusCode::REQUEST_TIMEOUT => Some(Reason::Deadline),
            StatusCode::PAYLOAD_TOO_LARGE => Some(Reason::BodyTooLarge),
            _ => None,
        },
    };
    if let Some(reason) = reason {
        app.counters.increment(reason);
    }
    apply(response.headers_mut());
    response
}
