//! Accounts, sessions, lockout and two-factor authentication (admin-auth; change
//! admin-auth).

pub mod commands;
pub mod credentials;
pub mod mail;
pub mod password;
pub mod recovery;
pub mod reset;
pub mod session;
pub mod totp;

use std::fmt;

use crate::db::migrate::DbFailure;
use crate::keys::RandomSourceError;

/// An account's role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Admin,
    Maintainer,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Maintainer => "maintainer",
        }
    }

    pub fn parse(text: &str) -> Option<Role> {
        match text {
            "admin" => Some(Role::Admin),
            "maintainer" => Some(Role::Maintainer),
            _ => None,
        }
    }

    /// Whether the role must sign in with a second factor (the admin; maintainers
    /// once `users-and-invites` lets the admin require it).
    pub fn requires_two_factor(self) -> bool {
        match self {
            Role::Admin => true,
            Role::Maintainer => false,
        }
    }
}

impl rusqlite::types::FromSql for Role {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Role> {
        let text = value.as_str()?;
        Role::parse(text).ok_or(rusqlite::types::FromSqlError::InvalidType)
    }
}

/// RFC 4648 base32 without padding.
pub fn base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let mut buffer = 0u16;
    let mut bits = 0;
    for &byte in bytes {
        buffer = (buffer << 8) | u16::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(ALPHABET[usize::from((buffer >> bits) & 31)]));
        }
    }
    if bits > 0 {
        out.push(char::from(
            ALPHABET[usize::from((buffer << (5 - bits)) & 31)],
        ));
    }
    out
}

/// A failure of the server's own, never of the request: answered 500 and logged
/// without detail.
#[derive(Debug)]
pub enum AuthError {
    Database(DbFailure),
    Random(RandomSourceError),
    /// A queued mail's recipient was refused; account mail names no address.
    Recipient,
}

impl From<rusqlite::Error> for AuthError {
    fn from(error: rusqlite::Error) -> AuthError {
        AuthError::Database(error.into())
    }
}

impl From<DbFailure> for AuthError {
    fn from(failure: DbFailure) -> AuthError {
        AuthError::Database(failure)
    }
}

impl From<RandomSourceError> for AuthError {
    fn from(error: RandomSourceError) -> AuthError {
        AuthError::Random(error)
    }
}

impl From<recovery::ReplaceError> for AuthError {
    fn from(error: recovery::ReplaceError) -> AuthError {
        match error {
            recovery::ReplaceError::Random(error) => AuthError::Random(error),
            recovery::ReplaceError::Database(error) => error.into(),
        }
    }
}

impl From<crate::mail::outbox::EnqueueError> for AuthError {
    fn from(error: crate::mail::outbox::EnqueueError) -> AuthError {
        match error {
            crate::mail::outbox::EnqueueError::Database(error) => error.into(),
            crate::mail::outbox::EnqueueError::InvalidRecipient => AuthError::Recipient,
        }
    }
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthError::Database(failure) => failure.fmt(f),
            AuthError::Random(error) => error.fmt(f),
            AuthError::Recipient => f.write_str("a queued mail's recipient was refused"),
        }
    }
}

impl std::error::Error for AuthError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_4648_base32_vectors() {
        for (input, expected) in [
            ("", ""),
            ("f", "MY"),
            ("fo", "MZXQ"),
            ("foo", "MZXW6"),
            ("foob", "MZXW6YQ"),
            ("fooba", "MZXW6YTB"),
            ("foobar", "MZXW6YTBOI"),
        ] {
            assert_eq!(base32(input.as_bytes()), expected, "{input}");
        }
        assert_eq!(base32(&[0xff; 10]), "7".repeat(16));
    }
}
