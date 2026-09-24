//! The multipart rule (http-security: Multipart bodies rejected unless explicitly
//! accepted). Body caps are tower-http's `RequestBodyLimitLayer` plus axum's
//! `DefaultBodyLimit` at the same value (D31.3), applied in `routing::build`.

use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::logging::{Reason, Rejected};
use crate::routing::table::Multipart;

/// Whether any `Content-Type` names the `multipart` media type.
pub fn is_multipart(headers: &HeaderMap) -> bool {
    headers.get_all(header::CONTENT_TYPE).iter().any(|value| {
        let bytes = value.as_bytes();
        let media_type = bytes.split(|&b| b == b';').next().unwrap_or_default();
        let trimmed = media_type.trim_ascii();
        trimmed.len() > b"multipart/".len()
            && trimmed[..b"multipart/".len()].eq_ignore_ascii_case(b"multipart/")
    })
}

/// Per-entry layer: 415 before any body byte is read.
pub async fn multipart_layer(
    State(multipart): State<Multipart>,
    request: Request,
    next: Next,
) -> Response {
    match multipart {
        Multipart::Rejected if is_multipart(request.headers()) => {
            let mut response = StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
            response
                .extensions_mut()
                .insert(Rejected(Reason::Multipart));
            response
        }
        Multipart::Rejected => next.run(request).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn multipart_media_types() {
        let check = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(header::CONTENT_TYPE, HeaderValue::from_str(value).unwrap());
            is_multipart(&headers)
        };
        for value in [
            "multipart/form-data; boundary=x",
            "Multipart/Form-Data",
            "MULTIPART/MIXED",
            "multipart/related",
            " multipart/x ; a=b",
        ] {
            assert!(check(value), "{value}");
        }
        for value in [
            "application/json",
            "text/multipart",
            "multipart",
            "multipart/",
            "application/x-www-form-urlencoded",
        ] {
            assert!(!check(value), "{value}");
        }
    }
}
