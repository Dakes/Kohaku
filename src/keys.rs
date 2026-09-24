//! The instance secret and, from task 3, the key registry (change foundation D6).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// Minimum decoded length of `KOHAKU_SECRET` (configuration: Instance secret).
pub const INSTANCE_SECRET_MIN_BYTES: usize = 32;

/// The decoded `KOHAKU_SECRET`: only ever an HMAC-SHA256 key. Deliberately implements
/// neither `Debug` nor `Display`, so it cannot reach a log line or an error by accident.
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

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(bytes: &[u8]) -> String {
        STANDARD.encode(bytes)
    }

    #[test]
    fn secret_encoding() {
        let bytes: Vec<u8> = (0u8..32)
            .map(|b| b.wrapping_mul(37).wrapping_add(11))
            .collect();
        let text = b64(&bytes);
        assert_eq!(
            InstanceSecret::from_base64(&text).map(|s| s.bytes().to_vec()),
            Ok(bytes.clone())
        );

        let url_safe = base64::engine::general_purpose::URL_SAFE.encode([0xfb_u8; 32]);
        let unpadded = base64::engine::general_purpose::STANDARD_NO_PAD.encode(&bytes);
        let long = b64(&[7u8; 64]);
        let wrapped = format!(
            "{}\n{}",
            &long[..76.min(long.len() - 4)],
            &long[76.min(long.len() - 4)..]
        );
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
}
