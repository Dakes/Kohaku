//! The admin area on the main host: sign-in, the signed-in home, the account page and
//! password reset (admin-auth; change admin-auth D3, D12).

pub mod account;
pub mod login;
pub mod reset;

use askama::Template;
use axum::Extension;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::auth::AuthError;
use crate::auth::session::{LOGIN_PATH, SessionUser, expired_session_cookie, see_other};
use crate::pages::{Chrome, html};
use crate::routing::AppState;

/// What `admin_base.html` shows of the signed-in account.
pub struct Nav {
    pub email: String,
    pub csrf: String,
}

impl Nav {
    pub fn of(user: &SessionUser) -> Nav {
        Nav {
            email: user.email.clone(),
            csrf: user.csrf_field(),
        }
    }
}

/// A failure of the server's own: logged without detail, answered 500.
pub fn server_error(error: &AuthError) -> Response {
    tracing::error!("admin request failed: {error}");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

#[derive(Template)]
#[template(path = "home.html")]
struct Home {
    chrome: Chrome,
    nav: Nav,
    role: &'static str,
    totp_enrolled: bool,
}

/// `GET /admin`: the signed-in home.
pub async fn home(Extension(user): Extension<SessionUser>) -> Response {
    html(&Home {
        chrome: Chrome::new(),
        nav: Nav::of(&user),
        role: user.role.as_str(),
        totp_enrolled: user.totp_enrolled,
    })
}

/// `POST /admin/logout`: deletes the session and expires its cookie.
pub async fn logout(
    State(app): State<AppState>,
    Extension(user): Extension<SessionUser>,
) -> Response {
    let id = user.session_id;
    let deleted = app
        .db
        .write(move |tx| {
            tx.execute("DELETE FROM sessions WHERE id = ?1", [id])
                .map_err(AuthError::from)
        })
        .await;
    if let Err(error) = deleted {
        return server_error(&error);
    }
    let mut response = see_other(LOGIN_PATH);
    response
        .headers_mut()
        .insert(header::SET_COOKIE, expired_session_cookie());
    response
}

/// Every other `/admin` path, reached only with a session.
pub async fn not_found() -> StatusCode {
    StatusCode::NOT_FOUND
}
