//! Server-rendered pages (host-routing: Main-host routes; change foundation D22).

use askama::Template;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

/// What every page's `base.html` needs.
pub struct Chrome {
    pub stylesheet: &'static str,
    /// Only a `dev` build loads the live-reload script.
    pub dev_reload: bool,
}

impl Chrome {
    pub fn new() -> Chrome {
        Chrome {
            stylesheet: crate::assets::path("kohaku.css"),
            dev_reload: cfg!(feature = "dev"),
        }
    }
}

impl Default for Chrome {
    fn default() -> Chrome {
        Chrome::new()
    }
}

#[derive(Template)]
#[template(path = "landing.html")]
struct Landing {
    chrome: Chrome,
}

/// Renders a template as a 200 HTML page.
pub fn html(template: &impl Template) -> Response {
    match template.render() {
        Ok(html) => ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response(),
        Err(_) => {
            tracing::error!("template failed to render");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `GET /` on the main host: static text, identical for every request.
pub async fn landing() -> Response {
    html(&Landing {
        chrome: Chrome::new(),
    })
}

/// Every unmatched path, on every method: the error-body layer writes the page.
pub async fn not_found() -> StatusCode {
    StatusCode::NOT_FOUND
}
