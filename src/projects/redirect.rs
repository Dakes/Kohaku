//! `/p/{slug}/…` on the main host: a 308 to the project's custom domain (projects:
//! Canonical redirect to the custom domain; change projects D5).

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};

use crate::routing::AppState;
use crate::routing::urls::Urls;

/// `GET` and `HEAD`: the redirect when the slug's project has a custom domain and the
/// path is not under `api`; else the 404 of an unmatched path, until later changes
/// serve these paths on the main host.
pub async fn canonical(State(app): State<AppState>, uri: Uri) -> Response {
    // The raw path, so the location keeps the request's own encoding.
    let Some(after) = uri.path().strip_prefix("/p/") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let (slug, rest) = after.split_at(after.find('/').unwrap_or(after.len()));
    if rest == "/api" || rest.starts_with("/api/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let map = app.host_map.current();
    let Some(host) = map.domain(slug) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let path = if rest.is_empty() { "/" } else { rest };
    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    let location = format!("{}{path}{query}", Urls::project_origin(host));
    match HeaderValue::try_from(location) {
        Ok(location) => (
            StatusCode::PERMANENT_REDIRECT,
            [(header::LOCATION, location)],
        )
            .into_response(),
        Err(_) => StatusCode::BAD_REQUEST.into_response(),
    }
}
