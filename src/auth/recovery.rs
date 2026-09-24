//! Recovery codes: 10 per enrollment, 80 random bits each, stored as SHA-256 only
//! (admin-auth: Two-factor authentication; change admin-auth D8).

use rusqlite::{Transaction, params};

use crate::keys::{RandomSourceError, random_bytes, sha256};

/// Codes per enrollment or regeneration.
pub const COUNT: usize = 10;

/// Base32 characters of a code (80 bits).
pub const LENGTH: usize = 16;

/// A code as shown once: `XXXX-XXXX-XXXX-XXXX`.
pub fn display(code: &str) -> String {
    let groups: Vec<&str> = (0..LENGTH / 4).map(|i| &code[i * 4..i * 4 + 4]).collect();
    groups.join("-")
}

/// Typed input as a code: uppercased, `-` and spaces removed, then exactly 16 base32
/// characters; `None` otherwise.
pub fn normalize(input: &str) -> Option<String> {
    let code: String = input
        .chars()
        .filter(|&c| c != '-' && c != ' ')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let base32 = |c: char| c.is_ascii_uppercase() || ('2'..='7').contains(&c);
    (code.chars().count() == LENGTH && code.chars().all(base32)).then_some(code)
}

/// Replaces every code of `user_id` with 10 new ones, in `tx`; returns them for
/// showing once.
pub fn replace(tx: &Transaction<'_>, user_id: i64) -> Result<Vec<String>, ReplaceError> {
    tx.execute("DELETE FROM recovery_codes WHERE user_id = ?1", [user_id])?;
    let mut codes = Vec::with_capacity(COUNT);
    while codes.len() < COUNT {
        let mut bytes = [0u8; LENGTH * 5 / 8];
        random_bytes(&mut bytes)?;
        let code = super::base32(&bytes);
        // INSERT OR IGNORE: a repeated code (2^-80) is drawn again, never stored twice.
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO recovery_codes (user_id, code_hash) VALUES (?1, ?2)",
            params![user_id, sha256(code.as_bytes()).as_slice()],
        )?;
        if inserted == 1 {
            codes.push(code);
        }
    }
    Ok(codes)
}

/// Consumes `code` (normalized) of `user_id` by one conditional delete; `true` only for
/// the one request that deleted it.
pub fn consume(tx: &Transaction<'_>, user_id: i64, code: &str) -> rusqlite::Result<bool> {
    crate::db::consume_one(
        tx,
        "DELETE FROM recovery_codes WHERE user_id = ?1 AND code_hash = ?2",
        params![user_id, sha256(code.as_bytes()).as_slice()],
    )
}

#[derive(Debug)]
pub enum ReplaceError {
    Random(RandomSourceError),
    Database(rusqlite::Error),
}

impl From<RandomSourceError> for ReplaceError {
    fn from(error: RandomSourceError) -> ReplaceError {
        ReplaceError::Random(error)
    }
}

impl From<rusqlite::Error> for ReplaceError {
    fn from(error: rusqlite::Error) -> ReplaceError {
        ReplaceError::Database(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_forms() {
        let code = "ABCDEFGHIJKLMN23";
        assert_eq!(display(code), "ABCD-EFGH-IJKL-MN23");
        for input in [
            "ABCD-EFGH-IJKL-MN23",
            "abcd-efgh-ijkl-mn23",
            "abcdefghijklmn23",
            "ABCD EFGH IJKL MN23",
            " abcd-EFGH-ijkl-MN23 ",
        ] {
            assert_eq!(normalize(input).as_deref(), Some(code), "{input}");
        }
        for bad in [
            "ABCD-EFGH-IJKL-MN2",
            "ABCD-EFGH-IJKL-MN231",
            "ABCD-EFGH-IJKL-MN21",
            "ABCD-EFGH-IJKL-MN2\u{212A}",
            "ABCD_EFGH_IJKL_MN23",
            "",
        ] {
            assert_eq!(normalize(bad), None, "{bad}");
        }
    }
}
