//! Outbound mail: text rules, the message builder and the delivery interface
//! (mail-outbox; change foundation D17, D18).

#[cfg(feature = "dev")]
pub mod dev;
pub mod outbox;
pub mod smtp;

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;

use lettre::Message;
use lettre::message::header::ContentType;
use lettre::message::{Mailbox, MessageBuilder};

use crate::config::SenderAddress;

/// Longest subject or quoted text, in Unicode scalar values.
pub const TEXT_CAP: usize = 200;

/// Longest recipient address.
pub const ADDRESS_CAP: usize = 254;

/// Cuts `text` to `TEXT_CAP` characters: its first 199 plus `…`.
fn cap(text: String) -> String {
    if text.chars().count() <= TEXT_CAP {
        return text;
    }
    let mut cut: String = text.chars().take(TEXT_CAP - 1).collect();
    cut.push('…');
    cut
}

fn is_line_break_or_control(c: char) -> bool {
    c.is_control() || c == '\u{2028}' || c == '\u{2029}'
}

/// The one `Subject` header value: control characters, U+2028 and U+2029 removed, then
/// capped.
pub fn subject(text: &str) -> String {
    cap(text
        .chars()
        .filter(|&c| !is_line_break_or_control(c))
        .collect())
}

/// Steps 1 to 3 of the user-text transform: breaks become one space, bidirectional
/// controls go, then the cap.
fn single_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_break = false;
    for c in text.chars() {
        if is_line_break_or_control(c) {
            if !in_break {
                out.push(' ');
            }
            in_break = true;
            continue;
        }
        in_break = false;
        let bidi = matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}');
        if !bidi {
            out.push(c);
        }
    }
    cap(out)
}

/// User-supplied text for a subject: single line, capped, in `"` with inner `"` as `'`.
pub fn quote_for_subject(text: &str) -> String {
    format!("\"{}\"", single_line(text).replace('"', "'"))
}

/// User-supplied text for a body: single line, capped, alone on a line after `> `.
pub fn quote_for_body(text: &str) -> String {
    format!("> {}", single_line(text))
}

/// A recipient refused before anything is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidRecipient;

impl fmt::Display for InvalidRecipient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid recipient address")
    }
}

impl std::error::Error for InvalidRecipient {}

/// At most 254 characters, exactly one `@`, no whitespace or control characters, and
/// an address lettre accepts, so every stored row can be built.
pub fn validate_recipient(address: &str) -> Result<(), InvalidRecipient> {
    let plain = address.chars().count() <= ADDRESS_CAP
        && address.matches('@').count() == 1
        && !address.chars().any(|c| c.is_whitespace() || c.is_control());
    if plain && lettre::Address::from_str(address).is_ok() {
        Ok(())
    } else {
        Err(InvalidRecipient)
    }
}

/// One `text/plain; charset=utf-8` part from the configured sender to one bare
/// address: no display name, `Cc`, `Bcc` or `Reply-To`.
pub fn build_message(
    sender: &SenderAddress,
    to: &str,
    subject_text: &str,
    body: &str,
) -> Result<Message, BuildFailure> {
    let from = lettre::Address::from_str(sender.as_str()).map_err(|_| BuildFailure)?;
    let to = lettre::Address::from_str(to).map_err(|_| BuildFailure)?;
    MessageBuilder::new()
        .from(Mailbox::new(None, from))
        .to(Mailbox::new(None, to))
        .subject(subject(subject_text))
        .header(ContentType::TEXT_PLAIN)
        .body(body.to_owned())
        .map_err(|_| BuildFailure)
}

/// A message that cannot be built; carries nothing about its content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildFailure;

/// A message ready for delivery, with its body as readable text for the `dev` printer.
pub struct Outgoing {
    pub row_id: i64,
    pub message: Message,
    pub body: String,
}

/// Why an attempt failed: a fixed class and the SMTP reply code, never reply text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Failure {
    pub class: FailureClass,
    pub code: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    Connection,
    Tls,
    Protocol,
    Authentication,
    Rejected,
    Temporary,
    Timeout,
    Build,
    Panic,
}

impl FailureClass {
    pub fn name(self) -> &'static str {
        match self {
            FailureClass::Connection => "connection",
            FailureClass::Tls => "tls",
            FailureClass::Protocol => "protocol",
            FailureClass::Authentication => "authentication",
            FailureClass::Rejected => "rejected",
            FailureClass::Temporary => "temporary",
            FailureClass::Timeout => "timeout",
            FailureClass::Build => "build",
            FailureClass::Panic => "panic",
        }
    }
}

pub type SendFuture<'a> = Pin<Box<dyn Future<Output = Result<(), Failure>> + Send + 'a>>;

/// Delivers one message: `Ok` only when the server accepted its data with a 2xx reply.
pub trait Mailer: Send + Sync + 'static {
    fn send(&self, outgoing: Outgoing) -> SendFuture<'_>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sender() -> SenderAddress {
        crate::config::parse_sender_for_tests("kohaku@kohaku.example.org")
    }

    #[test]
    fn subject_cut() {
        assert_eq!(subject("a\r\nb\u{2028}c\u{7}"), "abc");
        let long: String = "𝄞".repeat(300);
        let cut = subject(&long);
        assert_eq!(cut.chars().count(), 200);
        assert!(cut.ends_with('…'));
        assert_eq!(subject(&"x".repeat(200)), "x".repeat(200));
    }

    #[test]
    fn line_breaks_and_markup_cannot_forge_layout() {
        let title = "<a href=\"https://evil.example\">Crash</a>\n\nYour password expired. Log in at https://evil.example";
        assert_eq!(
            quote_for_body(title),
            "> <a href=\"https://evil.example\">Crash</a> Your password expired. Log in at https://evil.example"
        );
        assert!(!quote_for_body("a\r\n\u{2029}b").contains(['\r', '\n']));
    }

    #[test]
    fn quoted_text_cannot_close_its_quotation() {
        let text = "x\" is fixed. Urgent: \u{202E}verify your account \"";
        assert_eq!(
            quote_for_subject(text),
            "\"x' is fixed. Urgent: verify your account '\""
        );
        let long = "y".repeat(500);
        assert_eq!(quote_for_body(&long), format!("> {}…", "y".repeat(199)));
        for bidi in [
            '\u{061C}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{2066}', '\u{2069}',
        ] {
            assert_eq!(quote_for_body(&format!("a{bidi}b")), "> ab");
        }
    }

    #[test]
    fn header_injection_through_the_recipient_is_refused() {
        assert!(validate_recipient("victim@example.com\r\nBcc: list@example.net").is_err());
        let domain = format!(
            "{}.{}.{}.{}",
            "b".repeat(63),
            "c".repeat(63),
            "d".repeat(63),
            "e".repeat(57)
        );
        assert_eq!(format!("abcd@{domain}").len(), 254);
        assert!(validate_recipient(&format!("abcd@{domain}")).is_ok());
        assert!(validate_recipient(&format!("abcde@{domain}")).is_err());
        for bad in [
            "a b@example.com",
            "a@b@example.com",
            "example.com",
            "a@",
            "@x.test",
            "a\t@x.test",
        ] {
            assert!(validate_recipient(bad).is_err(), "{bad}");
        }
        assert!(validate_recipient("reporter@example.com").is_ok());
    }

    #[test]
    fn crlf_in_a_subject_cannot_add_headers() {
        let text = format!("\r\nBcc: list@example.net{}", "😀".repeat(300));
        let message = build_message(&sender(), "victim@example.com", &text, "body").unwrap();
        let formatted = String::from_utf8(message.formatted()).unwrap();
        let (head, _) = formatted.split_once("\r\n\r\n").unwrap();
        let unfolded = head.replace("\r\n ", " ").replace("\r\n\t", " ");
        let names: Vec<&str> = unfolded
            .lines()
            .map(|line| line.split(':').next().unwrap())
            .collect();
        for forbidden in ["Bcc", "Cc", "Reply-To"] {
            assert!(
                !names.iter().any(|n| n.eq_ignore_ascii_case(forbidden)),
                "{unfolded}"
            );
        }
        assert_eq!(
            names
                .iter()
                .filter(|n| n.eq_ignore_ascii_case("Subject"))
                .count(),
            1
        );
        assert!(unfolded.contains("Content-Type: text/plain; charset=utf-8"));
        assert!(unfolded.contains("From: kohaku@kohaku.example.org"));
        assert!(unfolded.contains("To: victim@example.com"));
        let expected = subject(&text);
        assert_eq!(expected.chars().count(), 200);
        assert!(expected.starts_with("Bcc: list@example.net"));
        let envelope = message.envelope();
        assert_eq!(envelope.to().len(), 1);
        assert_eq!(envelope.to()[0].to_string(), "victim@example.com");
        assert_eq!(
            envelope.from().unwrap().to_string(),
            "kohaku@kohaku.example.org"
        );
    }
}
