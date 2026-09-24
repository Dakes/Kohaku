//! Password reset by mail link, for maintainers (admin-auth: Password reset; change
//! admin-auth D9).

use askama::Template;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Form};
use rusqlite::{OptionalExtension, Transaction, params};
use serde::Deserialize;

use super::server_error;
use crate::audit::{Action, Actor, Target, audit};
use crate::auth::password::{check_password_rule, hash_password, normalize_email, valid_email};
use crate::auth::reset::issue_reset_token;
use crate::auth::session::end_all;
use crate::auth::{AuthError, Role, mail};
use crate::http::ClientAddr;
use crate::keys::{is_token_text, random_token, sha256};
use crate::limits::permits::{Bound, busy};
use crate::mail::outbox::{NewMail, Recipient, Wakeup, enqueue};
use crate::pages::{Chrome, html};
use crate::routing::AppState;
use crate::routing::urls::Urls;

/// The recipient of placeholder rows: never delivered, and no requester's address
/// is stored.
const PLACEHOLDER_ADDRESS: &str = "placeholder@kohaku.invalid";

#[derive(Template)]
#[template(path = "reset_request.html")]
struct RequestPage {
    chrome: Chrome,
}

#[derive(Template)]
#[template(path = "reset_sent.html")]
struct SentPage {
    chrome: Chrome,
}

#[derive(Template)]
#[template(path = "reset_form.html")]
struct FormPage {
    chrome: Chrome,
    token: String,
    error: Option<&'static str>,
}

#[derive(Template)]
#[template(path = "reset_invalid.html")]
struct InvalidPage {
    chrome: Chrome,
}

#[derive(Template)]
#[template(path = "reset_done.html")]
struct DonePage {
    chrome: Chrome,
}

fn invalid() -> Response {
    html(&InvalidPage {
        chrome: Chrome::new(),
    })
}

fn form_page(token: String, error: Option<&'static str>) -> Response {
    html(&FormPage {
        chrome: Chrome::new(),
        token,
        error,
    })
}

/// `GET /admin/reset`.
pub async fn request_form() -> Response {
    html(&RequestPage {
        chrome: Chrome::new(),
    })
}

#[derive(Deserialize)]
pub struct RequestForm {
    #[serde(default)]
    email: String,
}

/// `POST /admin/reset`: the same answer and the same database work for every address.
pub async fn request(
    State(app): State<AppState>,
    Extension(ClientAddr(client)): Extension<ClientAddr>,
    Form(form): Form<RequestForm>,
) -> Response {
    let email = normalize_email(&form.email);
    if !valid_email(&email) {
        return StatusCode::UNPROCESSABLE_ENTITY.into_response();
    }
    let now = app.clock.now();
    if !app.mail_address.take(&app.limiter, &email, now)
        || !app.mail_budget.take(&app.limiter, client, now)
    {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let Some(permit) = app.permits.try_public_write() else {
        return busy(Bound::PublicWrites);
    };
    let (wakeup, urls) = (app.outbox.clone(), app.urls.clone());
    let now = app.clock.unix();
    let queued = app
        .db
        .write(move |tx| {
            let _permit = permit;
            queue_reset(tx, &wakeup, &urls, &email, now)
        })
        .await;
    match queued {
        Ok(()) => html(&SentPage {
            chrome: Chrome::new(),
        }),
        Err(error) => server_error(&error),
    }
}

/// A reset token and its mail for an enabled maintainer with a password, otherwise a
/// placeholder row with a link of the same shape: the same queries either way.
fn queue_reset(
    tx: &Transaction<'_>,
    wakeup: &Wakeup,
    urls: &Urls,
    email: &str,
    now: i64,
) -> Result<(), AuthError> {
    let account: Option<i64> = tx
        .query_row(
            "SELECT id FROM users WHERE email = ?1 AND role = 'maintainer' AND disabled = 0
               AND password_hash IS NOT NULL",
            [email],
            |row| row.get(0),
        )
        .optional()?;
    let (text, recipient, placeholder) = match account {
        Some(id) => {
            let link = issue_reset_token(tx, urls, id, now)?;
            (
                mail::reset_link(urls.main_origin(), now, &link),
                Recipient::User(id),
                false,
            )
        }
        None => {
            tx.execute(
                "UPDATE tokens SET used_at = ?1
                 WHERE user_id IS NULL AND purpose = 'reset' AND used_at IS NULL",
                [now],
            )?;
            let (_, unused) = random_token()?;
            let link = urls.token_link(&format!("/admin/reset/{unused}"));
            let text = mail::reset_link(urls.main_origin(), now, &link);
            (
                text,
                Recipient::Address(PLACEHOLDER_ADDRESS.to_owned()),
                true,
            )
        }
    };
    enqueue(
        tx,
        wakeup,
        NewMail {
            kind: &mail::RESET_LINK,
            recipient,
            subject: text.subject,
            body: &text.body,
            placeholder,
        },
        now,
    )?;
    Ok(())
}

/// Whether `token` is a valid, unused reset token at `now`; read only.
async fn token_valid(app: &AppState, token: &str, now: i64) -> Result<bool, AuthError> {
    if !is_token_text(token) {
        return Ok(false);
    }
    let hash = sha256(token.as_bytes());
    app.db
        .read(move |conn| {
            conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM tokens t JOIN users u ON u.id = t.user_id
                     WHERE t.token_hash = ?1 AND t.purpose = 'reset' AND t.used_at IS NULL
                       AND t.expires_at > ?2 AND u.disabled = 0)",
                params![hash.as_slice(), now],
                |row| row.get(0),
            )
            .map_err(AuthError::from)
        })
        .await
}

/// `GET /admin/reset/{token}`: renders only, never consumes.
pub async fn link_form(State(app): State<AppState>, Path(token): Path<String>) -> Response {
    match token_valid(&app, &token, app.clock.unix()).await {
        Ok(true) => form_page(token, None),
        Ok(false) => invalid(),
        Err(error) => server_error(&error),
    }
}

#[derive(Deserialize)]
pub struct NewPassword {
    #[serde(default)]
    password: String,
    #[serde(default)]
    password_again: String,
}

/// `POST /admin/reset/{token}`: consumes the token in one conditional write, then sets
/// the password, ends every session and forgets device cookies. Creates no session.
pub async fn link_submit(
    State(app): State<AppState>,
    Path(token): Path<String>,
    Form(form): Form<NewPassword>,
) -> Response {
    match token_valid(&app, &token, app.clock.unix()).await {
        Ok(true) => {}
        Ok(false) => return invalid(),
        Err(error) => return server_error(&error),
    }
    if let Err(rule) = check_password_rule(&form.password) {
        return form_page(token, Some(rule.message()));
    }
    if form.password != form.password_again {
        return form_page(token, Some("The two passwords differ."));
    }
    let Some(permit) = app.permits.hashing(false).await else {
        return busy(Bound::Hashing);
    };
    let password = form.password;
    let hashed = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        hash_password(&password)
    })
    .await
    .expect("hashing does not panic");
    let hash = match hashed {
        Ok(hash) => hash,
        Err(error) => return server_error(&error.into()),
    };
    let (wakeup, origin) = (app.outbox.clone(), app.urls.main_origin().to_owned());
    let now = app.clock.unix();
    let token_hash = sha256(token.as_bytes());
    let consumed = app
        .db
        .write(move |tx| {
            let account: Option<(i64, Role)> = crate::db::consume_returning(
                tx,
                "UPDATE tokens SET used_at = ?2
                 WHERE token_hash = ?1 AND purpose = 'reset' AND used_at IS NULL
                   AND expires_at > ?2
                   AND user_id IN (SELECT id FROM users WHERE disabled = 0)
                 RETURNING user_id, (SELECT role FROM users WHERE id = user_id)",
                params![token_hash.as_slice(), now],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            let Some((id, role)) = account else {
                return Ok(false);
            };
            tx.execute(
                "UPDATE users SET password_hash = ?2 WHERE id = ?1",
                params![id, hash],
            )?;
            end_all(tx, id)?;
            tx.execute(
                "UPDATE tokens SET used_at = ?2
                 WHERE user_id = ?1 AND purpose = 'reset' AND used_at IS NULL",
                params![id, now],
            )?;
            let text = mail::password_changed(&origin, now);
            enqueue(
                tx,
                &wakeup,
                NewMail {
                    kind: &mail::PASSWORD_CHANGED,
                    recipient: Recipient::User(id),
                    subject: text.subject,
                    body: &text.body,
                    placeholder: false,
                },
                now,
            )?;
            audit(
                tx,
                Actor::User { id, role },
                Action::UserPasswordReset,
                Target::User(id),
                now,
            )?;
            Ok::<_, AuthError>(true)
        })
        .await;
    match consumed {
        Ok(true) => html(&DonePage {
            chrome: Chrome::new(),
        }),
        Ok(false) => invalid(),
        Err(error) => server_error(&error),
    }
}
