# Spec Delta

## MODIFIED Requirements

### Requirement: Mail is a single plain-text part

Every email SHALL be exactly one `text/plain; charset=utf-8` part, with no HTML, multipart structure or attachments.
- `From` and the envelope sender SHALL be the configured sender; `To` and the envelope SHALL name only the row's address, without display name; no `Cc`, `Bcc` or `Reply-To` SHALL be set. The only other headers a mail kind may add are
  `List-Unsubscribe` and `List-Unsubscribe-Post` with values built from the URL builder
  (reporter-verification).
- User-supplied text SHALL appear only in the subject and body. Every absolute URL SHALL come from the URL builder (configured hosts only, host-routing capability), never from request headers.
- The one `Subject` header SHALL have every control character (Unicode category Cc), U+2028 and U+2029 removed, and if then longer than 200 characters (Unicode scalar values) SHALL be cut to its first 199 plus `…` (U+2026), never splitting a character or failing.

#### Scenario: CR LF in a subject cannot add headers
- **WHEN** an attacker-supplied subject text is CR LF, `Bcc: list@example.net` and 300 four-byte UTF-8 characters
- **THEN** the message has one `text/plain; charset=utf-8` part, the configured `From`, the row's address as sole `To` and envelope recipient, no `Cc`, `Bcc` or `Reply-To`, and one valid UTF-8 `Subject` of 199 characters plus `…` holding `Bcc: list@example.net` only in its value
