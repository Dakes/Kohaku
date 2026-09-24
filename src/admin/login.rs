//! `GET` and `POST /admin/login` (admin-auth: Login; change admin-auth D5).

use askama::Template;
use axum::Form;
use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::response::Response;
use serde::Deserialize;

use super::server_error;
use crate::auth::credentials::{self, Attempt, Lookup, Outcome};
use crate::auth::password::normalize_email;
use crate::auth::session::{self, device_cookie, device_hash, see_other, session_cookie};
use crate::limits::permits::{Bound, busy};
use crate::pages::{Chrome, html};
use crate::routing::AppState;

/// Where a successful login goes: always this, never a location from the request.
const HOME: &str = "/admin";

#[derive(Template)]
#[template(path = "login.html")]
struct LoginPage {
    chrome: Chrome,
    failed: bool,
}

#[derive(Template)]
#[template(path = "login_2fa_required.html")]
struct TwoFactorRequired {
    chrome: Chrome,
}

#[derive(Deserialize)]
pub struct LoginForm {
    #[serde(default)]
    email: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    code: String,
}

fn page(failed: bool) -> Response {
    html(&LoginPage {
        chrome: Chrome::new(),
        failed,
    })
}

/// `GET /admin/login`: the form, or home for a signed-in user.
pub async fn form(State(app): State<AppState>, headers: HeaderMap) -> Response {
    match session::current(&app, &headers, app.clock.unix()).await {
        Ok(Some(_)) => see_other(HOME),
        Ok(None) => page(false),
        Err(failure) => server_error(&failure.into()),
    }
}

/// `POST /admin/login`: a new session only when every factor passes; every failure
/// gets the same page.
pub async fn submit(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    let device = device_hash(&headers);
    let attempt = Attempt {
        account: Lookup::Email(normalize_email(&form.email)),
        password: form.password,
        code: form.code,
        device,
    };
    let outcome = credentials::check(&app, attempt, move |tx, account, now| {
        let session = session::create(tx, account.id, now)?;
        // A login that already carried this account's device cookie keeps it.
        let device = match device {
            Some(hash) if session::is_known_device(tx, account.id, &hash, now)? => None,
            _ => Some(session::add_device(tx, account.id, now)?),
        };
        Ok((session, device))
    })
    .await;
    match outcome {
        Ok(Outcome::Success((token, device))) => {
            let mut response = see_other(HOME);
            let headers = response.headers_mut();
            headers.append(header::SET_COOKIE, session_cookie(&token));
            if let Some(device) = device {
                headers.append(header::SET_COOKIE, device_cookie(&device));
            }
            response
        }
        Ok(Outcome::Failure) => page(true),
        Ok(Outcome::TwoFactorRequired) => html(&TwoFactorRequired {
            chrome: Chrome::new(),
        }),
        Ok(Outcome::Busy) => busy(Bound::Hashing),
        Err(error) => server_error(&error),
    }
}
