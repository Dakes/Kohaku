//! `/healthz` and the TLS ask, matched before host routing (host-routing: Internal
//! endpoints are matched before host routing, TLS ask answers only for configured
//! project hosts; operations: Health endpoint).

use axum::extract::{RawQuery, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

use super::AppState;
use super::hosts::{HostEntry, normalize};

pub const HEALTHZ: &str = "/healthz";
pub const TLS_ASK: &str = "/.well-known/kohaku/tls-ask";

/// Answers from memory: binding only after startup makes 200 mean "started".
pub async fn healthz() -> Response {
    (
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"status":"ok"}"#,
    )
        .into_response()
}

/// 200 only for exactly one `domain` naming a project host of the in-memory map.
pub async fn tls_ask(State(app): State<AppState>, RawQuery(query): RawQuery) -> Response {
    let allowed = query
        .as_deref()
        .and_then(single_domain)
        .is_some_and(|domain| {
            matches!(
                app.host_map.current().get(&normalize(&domain)),
                Some(HostEntry::Project(_))
            )
        });
    let status = if allowed {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    };
    // One fixed body per status, whatever the name.
    (
        status,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "",
    )
        .into_response()
}

/// The percent-decoded value of the one `domain` parameter; `None` when it is missing,
/// empty, repeated or badly encoded.
fn single_domain(query: &str) -> Option<String> {
    let mut found = None;
    for pair in query.split('&') {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        if name != "domain" {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some(value);
    }
    let decoded = percent_decode(found?)?;
    (!decoded.is_empty()).then_some(decoded)
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            let hex = std::str::from_utf8(hex).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_parameter() {
        assert_eq!(
            single_domain("domain=Bugs.Example.NET:443").as_deref(),
            Some("Bugs.Example.NET:443")
        );
        assert_eq!(single_domain("x=1&domain=a%2Eb").as_deref(), Some("a.b"));
        for query in [
            "",
            "domain=",
            "domain",
            "domain=a&domain=b",
            "domain=%zz",
            "domain=%e2",
            "other=a",
        ] {
            assert_eq!(single_domain(query), None, "{query:?}");
        }
    }
}
