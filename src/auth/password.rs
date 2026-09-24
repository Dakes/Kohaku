//! Account emails, the password rule and argon2id hashing (admin-auth: Accounts).

use std::sync::OnceLock;

use argon2::password_hash::{PasswordHasher as _, PasswordVerifier as _};
use argon2::{Algorithm, Argon2, Params, Version};

use crate::keys::{RandomSourceError, random_bytes};

/// argon2id memory in KiB (19 MiB), iterations and lanes (OWASP's first choice).
pub const MEMORY_KIB: u32 = 19 * 1024;
pub const ITERATIONS: u32 = 2;
pub const PARALLELISM: u32 = 1;

/// Password length in Unicode scalar values (NIST 800-63B; no composition rules).
pub const PASSWORD_MIN: usize = 12;
pub const PASSWORD_MAX: usize = 1024;

/// Longest email address.
pub const EMAIL_MAX: usize = 254;

fn hasher() -> Argon2<'static> {
    let params = Params::new(MEMORY_KIB, ITERATIONS, PARALLELISM, None)
        .expect("the argon2 parameters are valid");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// An email as stored and looked up: surrounding ASCII whitespace removed, ASCII
/// letters lowercased.
pub fn normalize_email(email: &str) -> String {
    email
        .trim_matches(|c: char| c.is_ascii_whitespace())
        .to_ascii_lowercase()
}

/// Whether a normalized email follows the email rule and can be mailed to.
pub fn valid_email(email: &str) -> bool {
    crate::mail::validate_recipient(email).is_ok()
}

/// A new password breaks the length rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordRule {
    TooShort,
    TooLong,
}

impl PasswordRule {
    pub fn message(self) -> &'static str {
        match self {
            PasswordRule::TooShort => "The password needs at least 12 characters.",
            PasswordRule::TooLong => "The password may have at most 1024 characters.",
        }
    }
}

pub fn check_password_rule(password: &str) -> Result<(), PasswordRule> {
    match password.chars().count() {
        n if n < PASSWORD_MIN => Err(PasswordRule::TooShort),
        n if n > PASSWORD_MAX => Err(PasswordRule::TooLong),
        _ => Ok(()),
    }
}

/// An argon2id PHC string of `password` with a new random 16-byte salt. Blocking:
/// run under a hashing permit, off the async threads.
pub fn hash_password(password: &str) -> Result<String, RandomSourceError> {
    let mut salt = [0u8; 16];
    random_bytes(&mut salt)?;
    let hash = hasher()
        .hash_password_with_salt(password.as_bytes(), &salt)
        .expect("argon2 hashes any password of at most 1024 characters");
    Ok(hash.to_string())
}

/// Whether `password` matches the PHC string `hash`; a malformed hash matches nothing.
/// Blocking, like [`hash_password`].
pub fn verify_password(password: &str, hash: &str) -> bool {
    hasher().verify_password(password.as_bytes(), hash).is_ok()
}

static DUMMY_HASH: OnceLock<String> = OnceLock::new();

/// The hash checked when there is no account hash to check, so every failed login
/// does the same work: a random password under the production parameters, made once.
pub fn dummy_hash() -> Result<&'static str, RandomSourceError> {
    if let Some(hash) = DUMMY_HASH.get() {
        return Ok(hash);
    }
    let mut password = [0u8; 32];
    random_bytes(&mut password)?;
    let hash = hash_password(&crate::auth::base32(&password))?;
    Ok(DUMMY_HASH.get_or_init(|| hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emails_are_normalized() {
        assert_eq!(normalize_email(" Admin@Example.ORG\t"), "admin@example.org");
        assert_eq!(normalize_email("ÄDMIN@example.org"), "Ädmin@example.org");
        assert!(valid_email("admin@example.org"));
        for bad in ["a b@x.test", "a@b@x.test", "nobody", "a\r\n@x.test"] {
            assert!(!valid_email(bad), "{bad}");
        }
        assert!(!valid_email(&format!("{}@x.test", "a".repeat(250))));
    }

    #[test]
    fn password_rule_counts_scalar_values() {
        assert_eq!(
            check_password_rule(&"a".repeat(11)),
            Err(PasswordRule::TooShort)
        );
        assert_eq!(check_password_rule(&"a".repeat(12)), Ok(()));
        assert_eq!(check_password_rule(&"🔑".repeat(1024)), Ok(()));
        assert_eq!(
            check_password_rule(&"🔑".repeat(1025)),
            Err(PasswordRule::TooLong)
        );
        assert_eq!(check_password_rule("           !"), Ok(()));
    }

    #[test]
    fn hashes_are_argon2id_phc_strings_with_the_production_parameters() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(
            hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
            "{hash}"
        );
        assert!(!hash.contains("correct"));
        assert!(verify_password("correct horse battery staple", &hash));
        assert!(!verify_password("correct horse battery stapl", &hash));
        assert_ne!(hash, hash_password("correct horse battery staple").unwrap());
        assert!(!verify_password("x", "not a hash"));
        let dummy = dummy_hash().unwrap();
        assert!(dummy.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
        assert_eq!(dummy, dummy_hash().unwrap());
    }
}
