//! The account page and its changes, each behind re-authentication (admin-auth:
//! Re-authentication for sensitive account changes, Two-factor authentication).

use askama::Template;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use axum::{Extension, Form};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;

use super::{Nav, server_error};
use crate::audit::{Action, Actor, Target, audit};
use crate::auth::credentials::{self, Attempt, Lookup, Outcome};
use crate::auth::password::{check_password_rule, hash_password};
use crate::auth::session::{LOGIN_PATH, SessionUser, device_hash, end_all, see_other};
use crate::auth::totp::{NONCE_BYTES, QrSquares, Seed, key_uri, matching_step, qr_squares};
use crate::auth::{AuthError, mail, recovery};
use crate::keys::random_bytes;
use crate::limits::permits::{Bound, busy};
use crate::mail::outbox::{NewMail, Recipient, enqueue};
use crate::pages::{Chrome, html};
use crate::routing::AppState;

const PATH: &str = "/admin/account";

/// The generic re-authentication failure.
const REAUTH_FAILED: &str =
    "That did not work. Check your current password and code, then try again.";

#[derive(Template)]
#[template(path = "account.html")]
struct AccountPage {
    chrome: Chrome,
    nav: Nav,
    role: &'static str,
    totp_enrolled: bool,
    two_factor_required: bool,
    recovery_left: i64,
    error: Option<&'static str>,
    notice: Option<&'static str>,
}

#[derive(Template)]
#[template(path = "totp_enroll.html")]
struct EnrollPage {
    chrome: Chrome,
    nav: Nav,
    qr: QrSquares,
    seed: String,
    failed: bool,
}

#[derive(Template)]
#[template(path = "recovery_codes.html")]
struct CodesPage {
    chrome: Chrome,
    nav: Nav,
    intro: &'static str,
    codes: Vec<String>,
}

/// The account page, with an optional message; its state is read afresh.
async fn page(
    app: &AppState,
    user: &SessionUser,
    error: Option<&'static str>,
    notice: Option<&'static str>,
) -> Response {
    let id = user.user_id;
    let state = app
        .db
        .read(move |conn| {
            conn.query_row(
                "SELECT totp_nonce IS NOT NULL,
                     (SELECT count(*) FROM recovery_codes WHERE user_id = ?1)
                 FROM users WHERE id = ?1",
                [id],
                |row| Ok((row.get::<_, bool>(0)?, row.get::<_, i64>(1)?)),
            )
            .map_err(AuthError::from)
        })
        .await;
    match state {
        Ok((totp_enrolled, recovery_left)) => html(&AccountPage {
            chrome: Chrome::new(),
            nav: Nav::of(user),
            role: user.role.as_str(),
            totp_enrolled,
            two_factor_required: user.role.requires_two_factor(),
            recovery_left,
            error,
            notice,
        }),
        Err(error) => server_error(&error),
    }
}

fn codes_page(user: &SessionUser, intro: &'static str, codes: &[String]) -> Response {
    html(&CodesPage {
        chrome: Chrome::new(),
        nav: Nav::of(user),
        intro,
        codes: codes.iter().map(|code| recovery::display(code)).collect(),
    })
}

fn enroll_page(app: &AppState, user: &SessionUser, nonce: &[u8], failed: bool) -> Response {
    let seed = Seed::derive(&app.secret, user.user_id, nonce);
    let uri = key_uri(app.base_url.host().as_str(), &user.email, &seed);
    html(&EnrollPage {
        chrome: Chrome::new(),
        nav: Nav::of(user),
        qr: qr_squares(&uri),
        seed: seed.base32(),
        failed,
    })
}

/// `GET /admin/account`.
pub async fn show(
    State(app): State<AppState>,
    Extension(user): Extension<SessionUser>,
) -> Response {
    page(&app, &user, None, None).await
}

#[derive(Deserialize)]
pub struct Reauth {
    #[serde(default)]
    current_password: String,
    #[serde(default)]
    code: String,
}

impl Reauth {
    fn attempt(self, user: &SessionUser, headers: &HeaderMap) -> Attempt {
        Attempt {
            account: Lookup::Id(user.user_id),
            password: self.current_password,
            code: self.code,
            device: device_hash(headers),
        }
    }
}

/// A re-authenticated change: done with its value, or not, with the answer.
enum Reauthed<T> {
    Done(T),
    Refused(Response),
}

/// The account page with the generic message for a failed check, 503 or 500.
async fn reauthed<T>(
    app: &AppState,
    user: &SessionUser,
    outcome: Result<Outcome<T>, AuthError>,
) -> Reauthed<T> {
    match outcome {
        Ok(Outcome::Success(value)) => Reauthed::Done(value),
        Ok(Outcome::Failure | Outcome::TwoFactorRequired) => {
            Reauthed::Refused(page(app, user, Some(REAUTH_FAILED), None).await)
        }
        Ok(Outcome::Busy) => Reauthed::Refused(busy(Bound::Hashing)),
        Err(error) => Reauthed::Refused(server_error(&error)),
    }
}

#[derive(Deserialize)]
pub struct PasswordForm {
    #[serde(default)]
    current_password: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    new_password: String,
    #[serde(default)]
    new_password_again: String,
}

/// `POST /admin/account/password`: every session ends, the user signs in again.
pub async fn change_password(
    State(app): State<AppState>,
    Extension(user): Extension<SessionUser>,
    headers: HeaderMap,
    Form(form): Form<PasswordForm>,
) -> Response {
    if let Err(rule) = check_password_rule(&form.new_password) {
        return page(&app, &user, Some(rule.message()), None).await;
    }
    if form.new_password != form.new_password_again {
        return page(&app, &user, Some("The two new passwords differ."), None).await;
    }
    // Hashed first, under its own permit, so the change commits with the check.
    let Some(permit) = app.permits.hashing(false).await else {
        return busy(Bound::Hashing);
    };
    let new_password = form.new_password;
    let hashed = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        hash_password(&new_password)
    })
    .await
    .expect("hashing does not panic");
    let hash = match hashed {
        Ok(hash) => hash,
        Err(error) => return server_error(&error.into()),
    };
    let (wakeup, origin) = (app.outbox.clone(), app.urls.main_origin().to_owned());
    let reauth = Reauth {
        current_password: form.current_password,
        code: form.code,
    };
    let attempt = reauth.attempt(&user, &headers);
    let outcome = credentials::check(&app, attempt, move |tx, account, now| {
        tx.execute(
            "UPDATE users SET password_hash = ?2 WHERE id = ?1",
            params![account.id, hash],
        )?;
        end_all(tx, account.id)?;
        tx.execute(
            "UPDATE tokens SET used_at = ?2
             WHERE user_id = ?1 AND purpose = 'reset' AND used_at IS NULL",
            params![account.id, now],
        )?;
        let text = mail::password_changed(&origin, now);
        enqueue(
            tx,
            &wakeup,
            NewMail {
                kind: &mail::PASSWORD_CHANGED,
                recipient: Recipient::User(account.id),
                subject: text.subject,
                body: &text.body,
                placeholder: false,
            },
            now,
        )?;
        let actor = Actor::User {
            id: account.id,
            role: account.role,
        };
        audit(
            tx,
            actor,
            Action::UserPasswordChange,
            Target::User(account.id),
            now,
        )?;
        Ok(())
    })
    .await;
    match reauthed(&app, &user, outcome).await {
        Reauthed::Done(()) => see_other(LOGIN_PATH),
        Reauthed::Refused(response) => response,
    }
}

/// `POST /admin/account/totp`: starts (re-)enrollment with a new nonce, kept on this
/// session until confirmed.
pub async fn start_totp(
    State(app): State<AppState>,
    Extension(user): Extension<SessionUser>,
    headers: HeaderMap,
    Form(form): Form<Reauth>,
) -> Response {
    let session_id = user.session_id;
    let attempt = form.attempt(&user, &headers);
    let outcome = credentials::check(&app, attempt, move |tx, _, _| {
        let mut nonce = [0u8; NONCE_BYTES];
        random_bytes(&mut nonce)?;
        tx.execute(
            "UPDATE sessions SET pending_totp_nonce = ?2 WHERE id = ?1",
            params![session_id, nonce.as_slice()],
        )?;
        Ok(nonce)
    })
    .await;
    match reauthed(&app, &user, outcome).await {
        Reauthed::Done(nonce) => enroll_page(&app, &user, &nonce, false),
        Reauthed::Refused(response) => response,
    }
}

#[derive(Deserialize)]
pub struct CodeForm {
    #[serde(default)]
    code: String,
}

/// What confirming an enrollment found.
enum Confirmed {
    NotStarted,
    WrongCode(Vec<u8>),
    Enrolled(Vec<String>),
}

/// `POST /admin/account/totp/confirm`: enrolls the pending nonce once a code for it is
/// typed back; replaces recovery codes, ends other sessions, forgets device cookies.
pub async fn confirm_totp(
    State(app): State<AppState>,
    Extension(user): Extension<SessionUser>,
    Form(form): Form<CodeForm>,
) -> Response {
    let (session_id, user_id, role) = (user.session_id, user.user_id, user.role);
    let secret = std::sync::Arc::clone(&app.secret);
    let now = app.clock.unix();
    let confirmed = app
        .db
        .write(move |tx| {
            let pending: Option<Vec<u8>> = tx
                .query_row(
                    "SELECT pending_totp_nonce FROM sessions WHERE id = ?1",
                    [session_id],
                    |row| row.get(0),
                )
                .optional()?
                .flatten();
            let Some(nonce) = pending else {
                return Ok(Confirmed::NotStarted);
            };
            let seed = Seed::derive(&secret, user_id, &nonce);
            // The step is recorded as the new seed's last accepted one.
            let Some(step) = matching_step(&seed, form.code.trim(), now) else {
                return Ok(Confirmed::WrongCode(nonce));
            };
            tx.execute(
                "UPDATE users SET totp_nonce = ?2, totp_last_step = ?3 WHERE id = ?1",
                params![user_id, nonce, step],
            )?;
            let codes = recovery::replace(tx, user_id)?;
            tx.execute(
                "DELETE FROM sessions WHERE user_id = ?1 AND id != ?2",
                params![user_id, session_id],
            )?;
            tx.execute("DELETE FROM known_devices WHERE user_id = ?1", [user_id])?;
            tx.execute(
                "UPDATE sessions SET pending_totp_nonce = NULL WHERE id = ?1",
                [session_id],
            )?;
            let actor = Actor::User { id: user_id, role };
            audit(
                tx,
                actor,
                Action::UserTotpEnable,
                Target::User(user_id),
                now,
            )?;
            Ok::<_, AuthError>(Confirmed::Enrolled(codes))
        })
        .await;
    match confirmed {
        Ok(Confirmed::Enrolled(codes)) => codes_page(
            &user,
            "Two-factor login is on. Every other session was signed out.",
            &codes,
        ),
        Ok(Confirmed::WrongCode(nonce)) => enroll_page(&app, &user, &nonce, true),
        Ok(Confirmed::NotStarted) => page(&app, &user, Some("Start the setup again."), None).await,
        Err(error) => server_error(&error),
    }
}

/// `POST /admin/account/totp/disable`: refused for roles that require two factors.
pub async fn disable_totp(
    State(app): State<AppState>,
    Extension(user): Extension<SessionUser>,
    headers: HeaderMap,
    Form(form): Form<Reauth>,
) -> Response {
    if user.role.requires_two_factor() {
        return page(
            &app,
            &user,
            Some("Your role requires two-factor login."),
            None,
        )
        .await;
    }
    let attempt = form.attempt(&user, &headers);
    let outcome = credentials::check(&app, attempt, |tx, account, now| {
        tx.execute(
            "UPDATE users SET totp_nonce = NULL, totp_last_step = NULL WHERE id = ?1",
            [account.id],
        )?;
        tx.execute(
            "DELETE FROM recovery_codes WHERE user_id = ?1",
            [account.id],
        )?;
        tx.execute("DELETE FROM known_devices WHERE user_id = ?1", [account.id])?;
        let actor = Actor::User {
            id: account.id,
            role: account.role,
        };
        audit(
            tx,
            actor,
            Action::UserTotpDisable,
            Target::User(account.id),
            now,
        )?;
        Ok(())
    })
    .await;
    match reauthed(&app, &user, outcome).await {
        Reauthed::Done(()) => page(&app, &user, None, Some("Two-factor login is off.")).await,
        Reauthed::Refused(response) => response,
    }
}

/// `POST /admin/account/recovery-codes`: ten new codes replace the old ones.
pub async fn regenerate_codes(
    State(app): State<AppState>,
    Extension(user): Extension<SessionUser>,
    headers: HeaderMap,
    Form(form): Form<Reauth>,
) -> Response {
    if !user.totp_enrolled {
        return page(&app, &user, Some("Two-factor login is off."), None).await;
    }
    let attempt = form.attempt(&user, &headers);
    let outcome = credentials::check(&app, attempt, |tx, account, now| {
        let codes = recovery::replace(tx, account.id)?;
        let actor = Actor::User {
            id: account.id,
            role: account.role,
        };
        audit(
            tx,
            actor,
            Action::UserRecoveryCodes,
            Target::User(account.id),
            now,
        )?;
        Ok(codes)
    })
    .await;
    match reauthed(&app, &user, outcome).await {
        Reauthed::Done(codes) => {
            codes_page(&user, "Your old recovery codes no longer work.", &codes)
        }
        Reauthed::Refused(response) => response,
    }
}

/// `POST /admin/account/sessions/end-others`.
pub async fn end_other_sessions(
    State(app): State<AppState>,
    Extension(user): Extension<SessionUser>,
) -> Response {
    let (session_id, user_id, role) = (user.session_id, user.user_id, user.role);
    let now = app.clock.unix();
    let ended = app
        .db
        .write(move |tx| {
            tx.execute(
                "DELETE FROM sessions WHERE user_id = ?1 AND id != ?2",
                params![user_id, session_id],
            )?;
            let actor = Actor::User { id: user_id, role };
            audit(
                tx,
                actor,
                Action::UserSessionsEnd,
                Target::User(user_id),
                now,
            )?;
            Ok::<_, AuthError>(())
        })
        .await;
    match ended {
        Ok(()) => see_other(PATH),
        Err(error) => server_error(&error),
    }
}
