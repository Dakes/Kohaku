//! Error bodies: one fixed body per status and format, never echoing the request.

use askama::Template;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::pages::Chrome;
use crate::routing::table::ErrorFormat;

/// Marks an error response whose body the handler wrote itself (such as a form shown
/// again with its problem); the error-body layer keeps it.
#[derive(Debug, Clone, Copy)]
pub struct OwnBody;

#[derive(Template)]
#[template(path = "error.html")]
struct ErrorPage {
    chrome: Chrome,
    title: &'static str,
    text: &'static str,
}

fn describe(status: StatusCode) -> (&'static str, &'static str, &'static str) {
    match status.as_u16() {
        400 => (
            "bad_request",
            "Bad request",
            "The request could not be understood.",
        ),
        403 => ("forbidden", "Forbidden", "This request is not allowed."),
        404 => ("not_found", "Not found", "There is nothing here."),
        405 => (
            "method_not_allowed",
            "Method not allowed",
            "This address does not accept that method.",
        ),
        408 => ("timeout", "Request timeout", "The request took too long."),
        413 => ("too_large", "Too large", "The request is too large."),
        415 => (
            "unsupported_media_type",
            "Unsupported media type",
            "This content type is not accepted here.",
        ),
        421 => (
            "misdirected",
            "Misdirected request",
            "This server does not serve that host.",
        ),
        429 => (
            "too_many_requests",
            "Too many requests",
            "Too many requests. Try again later.",
        ),
        503 => ("busy", "Busy", "The server is busy. Try again later."),
        _ if status.is_server_error() => ("internal", "Server error", "Something went wrong."),
        _ => ("error", "Error", "The request failed."),
    }
}

/// The fixed error response for `status` in `format`.
pub fn response(status: StatusCode, format: ErrorFormat) -> Response {
    let (content_type, body) = render(status, format);
    let mut response = (status, body).into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}

fn render(status: StatusCode, format: ErrorFormat) -> (&'static str, Body) {
    let (code, title, text) = describe(status);
    match format {
        ErrorFormat::Html => {
            let page = ErrorPage {
                chrome: Chrome::new(),
                title,
                text,
            };
            let html = page.render().expect("the error template renders");
            ("text/html; charset=utf-8", Body::from(html))
        }
        ErrorFormat::Json => {
            let json = serde_json::json!({ "error": code, "message": text }).to_string();
            ("application/json", Body::from(json))
        }
    }
}

/// Per-entry layer: gives every error response the route's fixed error body, unless
/// its handler marked it [`OwnBody`].
pub async fn layer(State(format): State<ErrorFormat>, request: Request, next: Next) -> Response {
    let response = next.run(request).await;
    let status = response.status();
    if !(status.is_client_error() || status.is_server_error())
        || response.extensions().get::<OwnBody>().is_some()
    {
        return response;
    }
    let (mut parts, _) = response.into_parts();
    let (content_type, body) = render(status, format);
    parts.headers.remove(header::CONTENT_LENGTH);
    parts
        .headers
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    Response::from_parts(parts, body)
}
