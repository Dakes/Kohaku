//! Sessions, device cookies and CSRF tokens (admin-auth: Sessions, CSRF tokens; change
//! admin-auth D4).

use axum::body::{Body, Bytes};
use axum::extract::{FromRequest, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rusqlite::{OptionalExtension, Transaction, params};

use super::{AuthError, Role};
use crate::db::migrate::DbFailure;
use crate::keys::{KEY_BYTES, ct_eq, is_token_text, random_bytes, random_token, sha256};
use crate::routing::AppState;

pub const SESSION_COOKIE: &str = "__Host-kohaku_session";
pub const DEVICE_COOKIE: &str = "__Host-kohaku_device";

/// A session ends this long after its last recorded use...
pub const IDLE_SECONDS: i64 = 12 * 60 * 60;
/// ...and at the latest this long after it began.
pub const ABSOLUTE_SECONDS: i64 = 7 * 24 * 60 * 60;
/// Use is recorded at most this often, so reads rarely write.
pub const TOUCH_SECONDS: i64 = 5 * 60;

/// A device cookie's lifetime, and how long its row is kept.
pub const DEVICE_SECONDS: i64 = 365 * 24 * 60 * 60;
/// Device cookies kept per account; the oldest goes first.
pub const DEVICES_KEPT: i64 = 10;

/// Where a request without a session is sent.
pub const LOGIN_PATH: &str = "/admin/login";

/// The signed-in account of a request, set by [`layer`].
#[derive(Clone)]
pub struct SessionUser {
    pub session_id: i64,
    pub user_id: i64,
    pub role: Role,
    pub email: String,
    pub totp_enrolled: bool,
    csrf: [u8; KEY_BYTES],
}

impl SessionUser {
    /// The CSRF token as embedded in the account's forms.
    pub fn csrf_field(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.csrf)
    }
}

/// The one value of cookie `name` if it has the shape of a token; `None` when absent,
/// repeated or malformed, so nothing else reaches a query.
pub fn cookie_token<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut found = None;
    for value in headers.get_all(header::COOKIE) {
        let Ok(text) = value.to_str() else {
            continue;
        };
        for pair in text.split(';') {
            let Some((key, value)) = pair.trim_matches(' ').split_once('=') else {
                continue;
            };
            if key == name {
                if found.is_some() {
                    return None;
                }
                found = Some(value);
            }
        }
    }
    found.filter(|value| is_token_text(value))
}

/// The SHA-256 of a valid-looking device cookie, as stored.
pub fn device_hash(headers: &HeaderMap) -> Option<[u8; 32]> {
    cookie_token(headers, DEVICE_COOKIE).map(|token| sha256(token.as_bytes()))
}

/// The request's session, if valid at `now`; records its use at most every 5 minutes.
pub async fn current(
    app: &AppState,
    headers: &HeaderMap,
    now: i64,
) -> Result<Option<SessionUser>, DbFailure> {
    let Some(token) = cookie_token(headers, SESSION_COOKIE) else {
        return Ok(None);
    };
    let hash = sha256(token.as_bytes());
    let found = app
        .db
        .read(move |conn| {
            conn.query_row(
                "SELECT s.id, s.user_id, u.role, u.email, u.totp_nonce IS NOT NULL,
                     s.csrf_token, s.last_seen
                 FROM sessions s JOIN users u ON u.id = s.user_id
                 WHERE s.token_hash = ?1 AND u.password_hash IS NOT NULL AND u.disabled = 0
                   AND s.last_seen > ?2 - ?3 AND s.created_at > ?2 - ?4",
                params![hash.as_slice(), now, IDLE_SECONDS, ABSOLUTE_SECONDS],
                |row| {
                    let csrf: Vec<u8> = row.get(5)?;
                    Ok((
                        SessionUser {
                            session_id: row.get(0)?,
                            user_id: row.get(1)?,
                            role: row.get(2)?,
                            email: row.get(3)?,
                            totp_enrolled: row.get(4)?,
                            csrf: csrf.try_into().map_err(|_| rusqlite::Error::InvalidQuery)?,
                        },
                        row.get::<_, i64>(6)?,
                    ))
                },
            )
            .optional()
            .map_err(DbFailure::from)
        })
        .await?;
    let Some((user, last_seen)) = found else {
        return Ok(None);
    };
    if now - last_seen >= TOUCH_SECONDS {
        let id = user.session_id;
        app.db
            .write(move |tx| {
                tx.execute(
                    "UPDATE sessions SET last_seen = ?2 WHERE id = ?1 AND last_seen < ?2",
                    params![id, now],
                )
                .map_err(DbFailure::from)
            })
            .await?;
    }
    Ok(Some(user))
}

#[derive(serde::Deserialize)]
struct CsrfField {
    #[serde(default)]
    csrf: String,
}

/// Per-entry layer of every `Session` route: without a valid session, GET and HEAD
/// get `303` to the login page and anything else 403; with one, a state-changing
/// request must carry the session's CSRF token. The handler gets [`SessionUser`].
pub async fn layer(State(app): State<AppState>, request: Request, next: Next) -> Response {
    let now = app.clock.unix();
    let user = match current(&app, request.headers(), now).await {
        Ok(user) => user,
        Err(failure) => {
            tracing::error!("cannot read a session: {failure}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let safe = request.method() == Method::GET || request.method() == Method::HEAD;
    let Some(user) = user else {
        return if safe {
            see_other(LOGIN_PATH)
        } else {
            StatusCode::FORBIDDEN.into_response()
        };
    };
    let mut request = if safe {
        request
    } else {
        match check_csrf(request, &user.csrf).await {
            Some(request) => request,
            None => return StatusCode::FORBIDDEN.into_response(),
        }
    };
    request.extensions_mut().insert(user);
    next.run(request).await
}

/// Per-entry layer of every `Admin` route, inside [`layer`]: any account but the admin
/// gets the 404 of a nonexistent path.
pub async fn admin_layer(request: Request, next: Next) -> Response {
    match request.extensions().get::<SessionUser>() {
        Some(user) if user.role == Role::Admin => next.run(request).await,
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The request with its body put back when its form's `csrf` field is the session's
/// token; `None` otherwise. The body cap has already bounded the body.
async fn check_csrf(request: Request, expected: &[u8; KEY_BYTES]) -> Option<Request> {
    let (parts, body) = request.into_parts();
    let bytes: Bytes = axum::body::to_bytes(body, usize::MAX).await.ok()?;
    let probe = Request::from_parts(parts.clone(), Body::from(bytes.clone()));
    let axum::Form(field) = axum::Form::<CsrfField>::from_request(probe, &())
        .await
        .ok()?;
    let token = is_token_text(&field.csrf)
        .then(|| URL_SAFE_NO_PAD.decode(&field.csrf).ok())
        .flatten()?;
    ct_eq(&token, expected).then(|| Request::from_parts(parts, Body::from(bytes)))
}

/// `303 See Other` to a fixed path of this host.
pub fn see_other(path: &'static str) -> Response {
    (
        StatusCode::SEE_OTHER,
        [(header::LOCATION, HeaderValue::from_static(path))],
    )
        .into_response()
}

/// A new session of `user_id` in `tx`; returns the cookie value.
pub fn create(tx: &Transaction<'_>, user_id: i64, now: i64) -> Result<String, AuthError> {
    let (_, text) = random_token()?;
    let mut csrf = [0u8; KEY_BYTES];
    random_bytes(&mut csrf)?;
    tx.execute(
        "INSERT INTO sessions (token_hash, user_id, csrf_token, created_at, last_seen)
         VALUES (?1, ?2, ?3, ?4, ?4)",
        params![
            sha256(text.as_bytes()).as_slice(),
            user_id,
            csrf.as_slice(),
            now
        ],
    )?;
    Ok(text)
}

/// A new device cookie of `user_id` in `tx`, evicting the oldest beyond 10; returns
/// the cookie value.
pub fn add_device(tx: &Transaction<'_>, user_id: i64, now: i64) -> Result<String, AuthError> {
    let (_, text) = random_token()?;
    tx.execute(
        "INSERT INTO known_devices (user_id, token_hash, created_at) VALUES (?1, ?2, ?3)",
        params![user_id, sha256(text.as_bytes()).as_slice(), now],
    )?;
    tx.execute(
        "DELETE FROM known_devices WHERE user_id = ?1 AND id NOT IN (
             SELECT id FROM known_devices WHERE user_id = ?1
             ORDER BY created_at DESC, id DESC LIMIT ?2)",
        params![user_id, DEVICES_KEPT],
    )?;
    Ok(text)
}

/// Whether `hash` is a device cookie of `user_id` that has not expired at `now`.
pub fn is_known_device(
    conn: &rusqlite::Connection,
    user_id: i64,
    hash: &[u8; 32],
    now: i64,
) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM known_devices
             WHERE user_id = ?1 AND token_hash = ?2 AND created_at > ?3 - ?4)",
        params![user_id, hash.as_slice(), now, DEVICE_SECONDS],
        |row| row.get(0),
    )
}

/// Ends every session and forgets every device cookie of `user_id` (password set or
/// reset).
pub fn end_all(tx: &Transaction<'_>, user_id: i64) -> rusqlite::Result<()> {
    tx.execute("DELETE FROM sessions WHERE user_id = ?1", [user_id])?;
    tx.execute("DELETE FROM known_devices WHERE user_id = ?1", [user_id])?;
    Ok(())
}

fn cookie(name: &str, value: &str, attributes: &str) -> HeaderValue {
    HeaderValue::try_from(format!(
        "{name}={value}; Secure; HttpOnly; SameSite=Strict; Path=/{attributes}"
    ))
    .expect("cookie names and token values are visible ASCII")
}

/// `Set-Cookie` of a new session: no `Max-Age`, so it ends with the browser too.
pub fn session_cookie(token: &str) -> HeaderValue {
    cookie(SESSION_COOKIE, token, "")
}

/// `Set-Cookie` of a new device cookie, kept a year.
pub fn device_cookie(token: &str) -> HeaderValue {
    cookie(DEVICE_COOKIE, token, &format!("; Max-Age={DEVICE_SECONDS}"))
}

/// `Set-Cookie` expiring the session cookie (logout).
pub fn expired_session_cookie() -> HeaderValue {
    cookie(SESSION_COOKIE, "", "; Max-Age=0")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(values: &[&str]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for value in values {
            map.append(header::COOKIE, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn cookie_parsing() {
        let token = "A".repeat(43);
        let one = format!("{SESSION_COOKIE}={token}");
        assert_eq!(
            cookie_token(&headers(&[&one]), SESSION_COOKIE),
            Some(&*token)
        );
        let among = format!("x=1; {SESSION_COOKIE}={token}; y=2");
        assert_eq!(
            cookie_token(&headers(&[&among]), SESSION_COOKIE),
            Some(&*token)
        );
        assert_eq!(
            cookie_token(&headers(&["x=1", &one]), SESSION_COOKIE),
            Some(&*token)
        );
        for bad in [
            format!("{SESSION_COOKIE}={token}; {SESSION_COOKIE}={token}"),
            format!("{SESSION_COOKIE}=forged"),
            format!("{SESSION_COOKIE}={token}x"),
            format!("__host-kohaku_session={token}"),
            format!("{DEVICE_COOKIE}={token}"),
        ] {
            assert_eq!(
                cookie_token(&headers(&[&bad]), SESSION_COOKIE),
                None,
                "{bad}"
            );
        }
        let split = headers(&[&one, &one]);
        assert_eq!(cookie_token(&split, SESSION_COOKIE), None);
    }

    #[test]
    fn cookie_attributes() {
        assert_eq!(
            session_cookie("T"),
            "__Host-kohaku_session=T; Secure; HttpOnly; SameSite=Strict; Path=/"
        );
        assert_eq!(
            device_cookie("D"),
            "__Host-kohaku_device=D; Secure; HttpOnly; SameSite=Strict; Path=/; Max-Age=31536000"
        );
        assert_eq!(
            expired_session_cookie(),
            "__Host-kohaku_session=; Secure; HttpOnly; SameSite=Strict; Path=/; Max-Age=0"
        );
    }
}
