//! A `dev` build prints mail instead of sending it (mail-outbox: Development builds
//! print mail instead of sending it; change foundation D18). Not in release builds.

use std::io::Write;

use super::{Mailer, Outgoing, SendFuture};

/// Writes each message to stdout between marker lines and reports it accepted, so the
/// worker runs unchanged.
pub struct PrintMailer;

impl Mailer for PrintMailer {
    fn send(&self, outgoing: Outgoing) -> SendFuture<'_> {
        let text = render(&outgoing);
        Box::pin(async move {
            let mut stdout = std::io::stdout().lock();
            // A closed terminal loses the printout, not the row's outcome.
            let _ = stdout
                .write_all(text.as_bytes())
                .and_then(|()| stdout.flush());
            Ok(())
        })
    }
}

/// The printout: envelope, the headers as they would be sent, the readable body.
pub fn render(outgoing: &Outgoing) -> String {
    let envelope = outgoing.message.envelope();
    let from = envelope.from().map(ToString::to_string).unwrap_or_default();
    let to: Vec<String> = envelope.to().iter().map(ToString::to_string).collect();
    let headers = outgoing.message.headers().to_string();
    format!(
        "--- kohaku dev mail {id} ---\nEnvelope-From: {from}\nEnvelope-To: {to}\n{headers}\n{body}\n--- end ---\n",
        id = outgoing.row_id,
        to = to.join(", "),
        headers = headers.replace("\r\n", "\n").trim_end(),
        body = outgoing.body,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::build_message;

    #[test]
    fn developer_reads_a_one_time_code_in_the_terminal() {
        let sender = crate::config::parse_sender_for_tests("kohaku@localhost");
        let body = "Grüße! Your code is 424242.";
        let message = build_message(&sender, "dev@example.com", "Your code", body).unwrap();
        let text = render(&Outgoing {
            row_id: 7,
            message,
            body: body.to_owned(),
        });
        let inner = text
            .strip_prefix("--- kohaku dev mail 7 ---\n")
            .and_then(|t| t.strip_suffix("--- end ---\n"))
            .unwrap();
        for expected in [
            "From: kohaku@localhost",
            "To: dev@example.com",
            "Subject: Your code",
            "Content-Type: text/plain; charset=utf-8",
            "Grüße! Your code is 424242.",
            "Envelope-To: dev@example.com",
        ] {
            assert!(inner.contains(expected), "{expected}\n{text}");
        }
    }
}
