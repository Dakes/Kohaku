//! Reset tokens (admin-auth: Password reset, Account commands).

use rusqlite::{Transaction, params};

use super::AuthError;
use super::mail::RESET_LIFETIME;
use crate::keys::{random_token, sha256};
use crate::routing::urls::Urls;

/// A new 1-hour reset token of `user_id` in `tx`, after invalidating its others;
/// returns the link. Used by the reset form and `kohaku admin reset-password`.
pub fn issue_reset_token(
    tx: &Transaction<'_>,
    urls: &Urls,
    user_id: i64,
    now: i64,
) -> Result<String, AuthError> {
    tx.execute(
        "UPDATE tokens SET used_at = ?2
         WHERE user_id = ?1 AND purpose = 'reset' AND used_at IS NULL",
        params![user_id, now],
    )?;
    let (_, token) = random_token()?;
    tx.execute(
        "INSERT INTO tokens (purpose, token_hash, user_id, expires_at)
         VALUES ('reset', ?1, ?2, ?3)",
        params![
            sha256(token.as_bytes()).as_slice(),
            user_id,
            now + RESET_LIFETIME
        ],
    )?;
    Ok(urls.token_link(&format!("/admin/reset/{token}")))
}
