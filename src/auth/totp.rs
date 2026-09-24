//! RFC 6238 TOTP with seeds derived from the instance secret, the key URI and its QR
//! code (admin-auth: Two-factor authentication; change admin-auth D8).

use hmac::{Hmac, KeyInit, Mac};
use qrcodegen::{QrCode, QrCodeEcc};
use sha1::Sha1;

use crate::keys::{InstanceSecret, Purpose, ct_eq};

/// Seconds per step, counted from the Unix epoch.
pub const STEP_SECONDS: i64 = 30;

/// Digits of a code.
pub const DIGITS: usize = 6;

/// Bytes of a seed: the first 20 of the derived value (RFC 4226's recommended length).
pub const SEED_BYTES: usize = 20;

/// Bytes of an enrollment nonce.
pub const NONCE_BYTES: usize = 16;

/// Modules of light border around the code, as the QR standard requires.
pub const QUIET_ZONE: i32 = 4;

/// An account's TOTP seed. Derived on use and never stored; no `Debug` or `Display`.
pub struct Seed([u8; SEED_BYTES]);

impl Seed {
    /// The seed of account `user_id` enrolled with `nonce`.
    pub fn derive(secret: &InstanceSecret, user_id: i64, nonce: &[u8]) -> Seed {
        let full = secret.mac(Purpose::Totp, &[&user_id.to_be_bytes(), nonce]);
        let mut seed = [0u8; SEED_BYTES];
        seed.copy_from_slice(&full[..SEED_BYTES]);
        Seed(seed)
    }

    /// The seed as shown for manual entry: base32 without padding, 32 characters.
    pub fn base32(&self) -> String {
        super::base32(&self.0)
    }

    #[cfg(test)]
    fn from_bytes(bytes: [u8; SEED_BYTES]) -> Seed {
        Seed(bytes)
    }
}

/// The step containing unix time `now`.
pub fn step_at(now: i64) -> i64 {
    now.div_euclid(STEP_SECONDS)
}

/// The code of `step` (RFC 4226 HOTP with HMAC-SHA1, dynamic truncation, 6 digits).
pub fn code(seed: &Seed, step: i64) -> String {
    let mut mac = Hmac::<Sha1>::new_from_slice(&seed.0).expect("HMAC takes keys of any length");
    mac.update(&step.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = usize::from(digest[19] & 0x0f);
    let binary = u32::from_be_bytes([
        digest[offset] & 0x7f,
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ]);
    format!("{:06}", binary % 1_000_000)
}

/// Whether `text` has the shape of a TOTP code: exactly six ASCII digits.
pub fn is_code(text: &str) -> bool {
    text.len() == DIGITS && text.bytes().all(|b| b.is_ascii_digit())
}

/// The latest step among the current one and one either side whose code is `text`.
/// Accepting it still needs the replay write, which takes only a later step.
pub fn matching_step(seed: &Seed, text: &str, now: i64) -> Option<i64> {
    if !is_code(text) {
        return None;
    }
    let current = step_at(now);
    let mut found = None;
    for step in [current - 1, current, current + 1] {
        if ct_eq(code(seed, step).as_bytes(), text.as_bytes()) {
            found = Some(step);
        }
    }
    found
}

/// Percent-encodes everything but RFC 3986 unreserved characters.
fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The `otpauth://totp/` key URI authenticator apps scan: issuer `Kohaku (<main
/// host>)`, label `<issuer>:<email>`.
pub fn key_uri(main_host: &str, email: &str, seed: &Seed) -> String {
    let issuer = percent_encode(&format!("Kohaku ({main_host})"));
    format!(
        "otpauth://totp/{issuer}%3A{}?secret={}&issuer={issuer}&algorithm=SHA1&digits=6&period=30",
        percent_encode(email),
        seed.base32()
    )
}

/// A QR code as the page draws it: one unit square per dark module, offset by the
/// quiet zone, in a square of `size` units.
pub struct QrSquares {
    pub size: i32,
    pub dark: Vec<(i32, i32)>,
}

/// The QR code of `uri`, error correction level M or better at the same size.
pub fn qr_squares(uri: &str) -> QrSquares {
    let qr = QrCode::encode_text(uri, QrCodeEcc::Medium)
        .expect("a key URI is far below a QR code's capacity");
    let mut dark = Vec::new();
    for y in 0..qr.size() {
        for x in 0..qr.size() {
            if qr.get_module(x, y) {
                dark.push((x + QUIET_ZONE, y + QUIET_ZONE));
            }
        }
    }
    QrSquares {
        size: qr.size() + 2 * QUIET_ZONE,
        dark,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    fn secret(byte: u8) -> InstanceSecret {
        let text = base64::engine::general_purpose::STANDARD.encode([byte; 32]);
        InstanceSecret::from_base64(&text).unwrap()
    }

    #[test]
    fn rfc_6238_sha1_vectors() {
        let seed = Seed::from_bytes(*b"12345678901234567890");
        // RFC 6238 Appendix B lists 8 digits; a 6-digit code is their last six.
        for (time, expected) in [
            (59, "287082"),
            (1_111_111_109, "081804"),
            (1_111_111_111, "050471"),
            (1_234_567_890, "005924"),
            (2_000_000_000, "279037"),
            (20_000_000_000, "353130"),
        ] {
            assert_eq!(code(&seed, step_at(time)), expected, "T = {time}");
        }
        assert_eq!(seed.base32(), "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
    }

    #[test]
    fn window_is_one_step_either_side() {
        let seed = Seed::derive(&secret(1), 7, &[3; 16]);
        let now = 1_800_000_015;
        let step = step_at(now);
        for (offset, accepted) in [(-2, false), (-1, true), (0, true), (1, true), (2, false)] {
            let text = code(&seed, step + offset);
            let matched = matching_step(&seed, &text, now);
            if accepted {
                assert_eq!(matched, Some(step + offset), "offset {offset}");
            } else {
                assert!(
                    matched.is_none_or(|s| s != step + offset),
                    "offset {offset}"
                );
            }
        }
        assert_eq!(matching_step(&seed, "12345", now), None);
        assert_eq!(matching_step(&seed, "１２３４５６", now), None);
        assert_eq!(matching_step(&seed, " 123456", now), None);
    }

    #[test]
    fn seeds_depend_on_secret_account_and_nonce() {
        let base = Seed::derive(&secret(1), 7, &[3; 16]).base32();
        assert_eq!(base.len(), 32);
        assert_eq!(base, Seed::derive(&secret(1), 7, &[3; 16]).base32());
        assert_ne!(base, Seed::derive(&secret(2), 7, &[3; 16]).base32());
        assert_ne!(base, Seed::derive(&secret(1), 8, &[3; 16]).base32());
        assert_ne!(base, Seed::derive(&secret(1), 7, &[4; 16]).base32());
    }

    #[test]
    fn key_uri_format() {
        let seed = Seed::from_bytes(*b"12345678901234567890");
        assert_eq!(
            key_uri("kohaku.example.org", "a+b@example.org", &seed),
            "otpauth://totp/Kohaku%20%28kohaku.example.org%29%3Aa%2Bb%40example.org\
             ?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ\
             &issuer=Kohaku%20%28kohaku.example.org%29&algorithm=SHA1&digits=6&period=30"
        );
    }

    #[test]
    fn qr_squares_are_the_module_matrix() {
        let uri = key_uri(
            "kohaku.example.org",
            "admin@example.org",
            &Seed::from_bytes([9; 20]),
        );
        let squares = qr_squares(&uri);
        let qr = QrCode::encode_text(&uri, QrCodeEcc::Medium).unwrap();
        assert_eq!(squares.size, qr.size() + 8);
        assert!(qr.error_correction_level() >= QrCodeEcc::Medium);
        let dark: std::collections::HashSet<_> = squares.dark.iter().copied().collect();
        for y in -4..qr.size() + 4 {
            for x in -4..qr.size() + 4 {
                let module = (0..qr.size()).contains(&x)
                    && (0..qr.size()).contains(&y)
                    && qr.get_module(x, y);
                assert_eq!(dark.contains(&(x + 4, y + 4)), module, "({x}, {y})");
            }
        }
    }
}
