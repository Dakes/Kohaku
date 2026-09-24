//! The instance secret, per-boot keys and the one MAC encoding every key uses
//! (change foundation D6; design §5 Key inventory).

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Minimum decoded length of `KOHAKU_SECRET` (configuration: Instance secret).
pub const INSTANCE_SECRET_MIN_BYTES: usize = 32;

/// Length of a per-boot key and of every MAC.
pub const KEY_BYTES: usize = 32;

/// What a MAC is for. Each purpose has its own label, the first field of every MAC
/// input, so a value computed for one use never verifies for another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// The database's keycheck: shows whether the configured secret made the database.
    Keycheck,
    /// TOTP seeds, from the account id and its enrollment nonce (admin-auth).
    Totp,
    /// Keys of the per-address mail buckets, under a per-boot key (request-limits).
    MailAddress,
}

impl Purpose {
    /// Every purpose, for the uniqueness test.
    pub const ALL: &'static [Purpose] = &[Purpose::Keycheck, Purpose::Totp, Purpose::MailAddress];

    pub fn label(self) -> &'static str {
        match self {
            Purpose::Keycheck => "kohaku/keycheck",
            Purpose::Totp => "kohaku/totp",
            Purpose::MailAddress => "kohaku/mail-address",
        }
    }
}

/// HMAC-SHA256 under `key` of the purpose label followed by `fields`, each part
/// prefixed by its length as a big-endian u32, so no two inputs share an encoding.
fn mac(key: &[u8], purpose: Purpose, fields: &[&[u8]]) -> HmacSha256 {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC takes keys of any length");
    for part in std::iter::once(purpose.label().as_bytes()).chain(fields.iter().copied()) {
        let len = u32::try_from(part.len()).expect("MAC field longer than 4 GiB");
        mac.update(&len.to_be_bytes());
        mac.update(part);
    }
    mac
}

fn compute(key: &[u8], purpose: Purpose, fields: &[&[u8]]) -> [u8; KEY_BYTES] {
    mac(key, purpose, fields).finalize().into_bytes().into()
}

/// Constant-time check of `tag` against the MAC of `fields`.
fn verify(key: &[u8], purpose: Purpose, fields: &[&[u8]], tag: &[u8]) -> bool {
    mac(key, purpose, fields).verify_slice(tag).is_ok()
}

/// The decoded `KOHAKU_SECRET`: only ever an HMAC-SHA256 key. Deliberately implements
/// neither `Debug` nor `Display`, so it cannot reach a log line or an error by accident.
/// `Clone` only so the running server keeps its own copy for TOTP seeds.
///
/// ```compile_fail
/// let secret = kohaku::keys::InstanceSecret::from_base64("").unwrap();
/// println!("{secret:?}");
/// ```
///
/// ```compile_fail
/// let secret = kohaku::keys::InstanceSecret::from_base64("").unwrap();
/// println!("{secret}");
/// ```
#[derive(Clone)]
pub struct InstanceSecret(Vec<u8>);

/// `KOHAKU_SECRET` is not standard padded base64 of at least 32 bytes. Carries nothing
/// about the rejected value.
#[derive(Debug, PartialEq, Eq)]
pub struct InstanceSecretFormatError;

impl InstanceSecret {
    /// Decodes standard-alphabet base64 with `=` padding (RFC 4648 §4), taken exactly
    /// as given: whitespace, line breaks, URL-safe characters or missing padding fail.
    pub fn from_base64(text: &str) -> Result<Self, InstanceSecretFormatError> {
        let bytes = STANDARD
            .decode(text)
            .map_err(|_| InstanceSecretFormatError)?;
        if bytes.len() < INSTANCE_SECRET_MIN_BYTES {
            return Err(InstanceSecretFormatError);
        }
        Ok(Self(bytes))
    }

    pub fn mac(&self, purpose: Purpose, fields: &[&[u8]]) -> [u8; KEY_BYTES] {
        compute(&self.0, purpose, fields)
    }

    pub fn verify(&self, purpose: Purpose, fields: &[&[u8]], tag: &[u8]) -> bool {
        verify(&self.0, purpose, fields, tag)
    }

    /// The value stored in `meta.keycheck`.
    pub fn keycheck(&self) -> [u8; KEY_BYTES] {
        self.mac(Purpose::Keycheck, &[])
    }

    /// Whether `stored` is this secret's keycheck, in constant time.
    pub fn matches_keycheck(&self, stored: &[u8]) -> bool {
        self.verify(Purpose::Keycheck, &[], stored)
    }
}

/// A key living only in this process's memory, replaced on every start. No `Debug`,
/// `Display`, `Clone` or serialization.
///
/// ```compile_fail
/// let key = kohaku::keys::PerBootKey::generate().unwrap();
/// println!("{key:?}");
/// ```
///
/// ```compile_fail
/// let key = kohaku::keys::PerBootKey::generate().unwrap();
/// println!("{key}");
/// ```
pub struct PerBootKey([u8; KEY_BYTES]);

/// The operating system's random source failed; no key was made.
#[derive(Debug, PartialEq, Eq)]
pub struct RandomSourceError;

impl std::fmt::Display for RandomSourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the operating system's random source failed")
    }
}

impl std::error::Error for RandomSourceError {}

/// Whether `a` equals `b`, in time independent of where they differ (CSRF tokens).
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let difference = a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y));
    std::hint::black_box(difference) == 0
}

/// SHA-256 of `bytes`: the stored form of session, device, token and recovery values.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest as _;
    Sha256::digest(bytes).into()
}

/// A new random token of 256 bits, and its text: base64url without padding (43
/// characters).
pub fn random_token() -> Result<([u8; KEY_BYTES], String), RandomSourceError> {
    let mut bytes = [0u8; KEY_BYTES];
    random_bytes(&mut bytes)?;
    Ok((bytes, URL_SAFE_NO_PAD.encode(bytes)))
}

/// Length of a token's text.
pub const TOKEN_TEXT_LEN: usize = 43;

/// Whether `text` has the shape of a token: exactly 43 base64url characters. Checked
/// before any query, so other values never reach the database.
pub fn is_token_text(text: &str) -> bool {
    text.len() == TOKEN_TEXT_LEN
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        && URL_SAFE_NO_PAD.decode(text).is_ok()
}

/// Fills `buf` from the operating system's cryptographically secure random source.
pub fn random_bytes(buf: &mut [u8]) -> Result<(), RandomSourceError> {
    getrandom::fill(buf).map_err(|_| RandomSourceError)
}

impl PerBootKey {
    pub fn generate() -> Result<Self, RandomSourceError> {
        let mut key = [0u8; KEY_BYTES];
        random_bytes(&mut key)?;
        Ok(Self(key))
    }

    pub fn mac(&self, purpose: Purpose, fields: &[&[u8]]) -> [u8; KEY_BYTES] {
        compute(&self.0, purpose, fields)
    }

    pub fn verify(&self, purpose: Purpose, fields: &[&[u8]], tag: &[u8]) -> bool {
        verify(&self.0, purpose, fields, tag)
    }

    #[cfg(test)]
    pub(crate) fn bytes(&self) -> &[u8; KEY_BYTES] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn b64(bytes: &[u8]) -> String {
        STANDARD.encode(bytes)
    }

    fn secret(byte: u8) -> InstanceSecret {
        InstanceSecret::from_base64(&b64(&[byte; 32])).unwrap()
    }

    #[test]
    fn secret_encoding() {
        let bytes: Vec<u8> = (0u8..32)
            .map(|b| b.wrapping_mul(37).wrapping_add(11))
            .collect();
        let text = b64(&bytes);
        assert_eq!(
            InstanceSecret::from_base64(&text).map(|s| s.0.clone()),
            Ok(bytes.clone())
        );

        let url_safe = base64::engine::general_purpose::URL_SAFE.encode([0xfb_u8; 32]);
        let unpadded = base64::engine::general_purpose::STANDARD_NO_PAD.encode(&bytes);
        let long = b64(&[7u8; 64]);
        let wrapped = format!("{}\n{}", &long[..60], &long[60..]);
        let rejected = [
            format!("{text}\n"),
            format!(" {text}"),
            format!("{text} "),
            b64(&bytes[..31]),
            url_safe,
            unpadded,
            wrapped,
            String::new(),
        ];
        for value in &rejected {
            assert_eq!(
                InstanceSecret::from_base64(value).err(),
                Some(InstanceSecretFormatError),
                "{value:?}"
            );
        }
    }

    #[test]
    fn mac_inputs_cannot_collide() {
        let key = secret(1);
        let with = |fields: &[&[u8]]| key.mac(Purpose::Keycheck, fields);
        assert_ne!(with(&[b"ab", b"c"]), with(&[b"a", b"bc"]));
        // The spec's labels are not purposes, so build their inputs by hand.
        let raw = |label: &str, fields: &[&[u8]]| {
            let mut m = HmacSha256::new_from_slice(&key.0).unwrap();
            for part in std::iter::once(label.as_bytes()).chain(fields.iter().copied()) {
                m.update(&(part.len() as u32).to_be_bytes());
                m.update(part);
            }
            <[u8; 32]>::from(m.finalize().into_bytes())
        };
        assert_ne!(raw("kohaku/a", &[b"bc"]), raw("kohaku/ab", &[b"c"]));
        assert_ne!(
            raw("kohaku/x", &[b"ab", b"c"]),
            raw("kohaku/x", &[b"a", b"bc"])
        );
        // The production encoding is exactly `raw`.
        assert_eq!(with(&[b"ab", b"c"]), raw("kohaku/keycheck", &[b"ab", b"c"]));
        assert_ne!(with(&[]), with(&[b""]));
    }

    #[test]
    fn labels_are_unique_and_prefixed() {
        let labels: HashSet<&str> = Purpose::ALL.iter().map(|p| p.label()).collect();
        assert_eq!(labels.len(), Purpose::ALL.len());
        assert!(labels.iter().all(|l| l.starts_with("kohaku/")));
    }

    #[test]
    fn keycheck_is_stable_per_secret() {
        assert_eq!(secret(1).keycheck(), secret(1).keycheck());
        assert_ne!(secret(1).keycheck(), secret(2).keycheck());
        assert!(secret(1).matches_keycheck(&secret(1).keycheck()));
        assert!(!secret(1).matches_keycheck(&secret(2).keycheck()));
        assert!(!secret(1).matches_keycheck(&secret(1).keycheck()[..31]));
        assert!(!secret(1).matches_keycheck(&[]));
    }

    #[test]
    fn constant_time_comparison() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
        assert!(ct_eq(b"", b""));
    }

    #[test]
    fn token_shape() {
        let (bytes, text) = random_token().unwrap();
        assert!(is_token_text(&text));
        assert_eq!(URL_SAFE_NO_PAD.decode(&text).unwrap(), bytes);
        assert_ne!(random_token().unwrap().1, text);
        for bad in [
            "",
            &text[..42],
            &format!("{text}A"),
            &format!("{}+", &text[..42]),
            &format!("{}=", &text[..42]),
            &format!("{}é", &text[..41]),
        ] {
            assert!(!is_token_text(bad), "{bad}");
        }
    }

    #[test]
    fn per_boot_keys_differ() {
        let a = PerBootKey::generate().unwrap();
        let b = PerBootKey::generate().unwrap();
        assert_ne!(a.bytes(), b.bytes());
        assert_eq!(a.bytes().len(), 32);
        let tag = a.mac(Purpose::Keycheck, &[b"x"]);
        assert!(a.verify(Purpose::Keycheck, &[b"x"], &tag));
        assert!(!b.verify(Purpose::Keycheck, &[b"x"], &tag));
    }
}
