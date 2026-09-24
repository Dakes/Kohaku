//! Fetch-Metadata and Origin check, and admin resource isolation (http-security;
//! change foundation D14).

use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::RoutedHost;
use crate::logging::{Reason, Rejected};
use crate::routing::table::Headerless;

const SEC_FETCH_SITE: &str = "sec-fetch-site";
const SEC_FETCH_MODE: &str = "sec-fetch-mode";

/// The one value of a header; `Err` when it is repeated.
fn single<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a [u8]>, ()> {
    let mut values = headers.get_all(name).iter();
    let first = values.next().map(|v| v.as_bytes());
    if values.next().is_some() {
        return Err(());
    }
    Ok(first)
}

/// Whether a state-changing request may reach its route.
pub fn origin_allowed(headers: &HeaderMap, expected: &str, headerless: Headerless) -> bool {
    let (Ok(site), Ok(origin)) = (
        single(headers, SEC_FETCH_SITE),
        single(headers, header::ORIGIN.as_str()),
    ) else {
        return false;
    };
    if let Some(site) = site {
        return site == b"same-origin" && origin.is_none_or(|o| o == expected.as_bytes());
    }
    match origin {
        Some(origin) => origin == expected.as_bytes(),
        None => match headerless {
            Headerless::NotExempt => false,
            Headerless::Exempt => true,
        },
    }
}

fn reject(reason: Reason) -> Response {
    let mut response = StatusCode::FORBIDDEN.into_response();
    response.extensions_mut().insert(Rejected(reason));
    response
}

/// Per-entry layer: every method but GET and HEAD passes the check before anything
/// else reads the request.
pub async fn origin_layer(
    State(headerless): State<Headerless>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method();
    if method == Method::GET || method == Method::HEAD {
        return next.run(request).await;
    }
    let expected = request
        .extensions()
        .get::<RoutedHost>()
        .map(|host| host.origin.as_str());
    match expected {
        Some(expected) if origin_allowed(request.headers(), expected, headerless) => {
            next.run(request).await
        }
        _ => reject(Reason::CrossSite),
    }
}

/// Whether a main-host GET or HEAD for `/admin` or below passes resource isolation.
pub fn admin_isolation_allows(method: &Method, path: &str, headers: &HeaderMap) -> bool {
    let admin = path == "/admin" || path.starts_with("/admin/");
    if !admin || (method != Method::GET && method != Method::HEAD) {
        return true;
    }
    let Ok(site) = single(headers, SEC_FETCH_SITE) else {
        return false;
    };
    match site {
        None | Some(b"same-origin" | b"none") => true,
        Some(b"same-site" | b"cross-site") => {
            matches!(single(headers, SEC_FETCH_MODE), Ok(Some(b"navigate")))
        }
        Some(_) => false,
    }
}

/// The admin-isolation rejection.
pub fn admin_isolation_rejection() -> Response {
    reject(Reason::AdminIsolation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(
                axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    const ORIGIN: &str = "https://kohaku.example.org";

    #[test]
    fn origin_rules() {
        let ok =
            |pairs: &[(&str, &str)]| origin_allowed(&headers(pairs), ORIGIN, Headerless::NotExempt);
        assert!(ok(&[("origin", ORIGIN)]));
        assert!(ok(&[("origin", ORIGIN), ("sec-fetch-site", "same-origin")]));
        assert!(ok(&[("sec-fetch-site", "same-origin")]));
        for site in ["cross-site", "same-site", "none"] {
            assert!(!ok(&[("sec-fetch-site", site)]));
            assert!(!ok(&[("sec-fetch-site", site), ("origin", ORIGIN)]));
        }
        assert!(!ok(&[]));
        let exempt =
            |pairs: &[(&str, &str)]| origin_allowed(&headers(pairs), ORIGIN, Headerless::Exempt);
        assert!(exempt(&[]));
        assert!(!exempt(&[("origin", "null")]));
        assert!(!exempt(&[("sec-fetch-site", "cross-site")]));
        assert!(!ok(&[("origin", ORIGIN), ("origin", ORIGIN)]));
        assert!(!ok(&[
            ("sec-fetch-site", "same-origin"),
            ("sec-fetch-site", "cross-site")
        ]));
        for origin in [
            "null",
            "https://evil.example",
            "http://kohaku.example.org",
            "https://kohaku.example.org:443",
            "https://kohaku.example.org/",
            "https://KOHAKU.EXAMPLE.ORG",
            "https://kohaku.example.org.evil.example",
        ] {
            assert!(!ok(&[("origin", origin)]), "{origin}");
            assert!(
                !ok(&[("origin", origin), ("sec-fetch-site", "same-origin")]),
                "{origin}"
            );
        }
    }

    #[test]
    fn admin_isolation() {
        let allows = |path: &str, pairs: &[(&str, &str)]| {
            admin_isolation_allows(&Method::GET, path, &headers(pairs))
        };
        assert!(!allows(
            "/admin/anything",
            &[
                ("sec-fetch-site", "cross-site"),
                ("sec-fetch-mode", "no-cors")
            ]
        ));
        assert!(!allows(
            "/admin",
            &[("sec-fetch-site", "same-site"), ("sec-fetch-mode", "cors")]
        ));
        assert!(!allows("/admin", &[("sec-fetch-site", "evil")]));
        assert!(!allows(
            "/admin",
            &[
                ("sec-fetch-site", "same-origin"),
                ("sec-fetch-site", "cross-site")
            ]
        ));
        assert!(!allows(
            "/admin",
            &[
                ("sec-fetch-site", "cross-site"),
                ("sec-fetch-mode", "navigate"),
                ("sec-fetch-mode", "navigate")
            ]
        ));
        assert!(allows("/admin", &[("sec-fetch-site", "same-origin")]));
        assert!(allows("/admin", &[("sec-fetch-site", "none")]));
        assert!(allows("/admin", &[]));
        assert!(allows(
            "/admin",
            &[
                ("sec-fetch-site", "cross-site"),
                ("sec-fetch-mode", "navigate")
            ]
        ));
        assert!(allows("/administrator", &[("sec-fetch-site", "evil")]));
        assert!(admin_isolation_allows(
            &Method::POST,
            "/admin",
            &headers(&[("sec-fetch-site", "evil")])
        ));
    }
}
