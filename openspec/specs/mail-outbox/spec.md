# mail-outbox Specification

## Purpose
Outbound mail: a persistent, prioritized and retried outbox delivered as plain text to an external SMTP server over verified TLS (printed to the terminal instead in a `dev` build), so that no request waits on mail, tokens do not outlive their delivery and user text cannot forge headers or layout (design baseline §9).

## Requirements

### Requirement: Mail is sent only through the persistent outbox

Every outbound email SHALL first be stored as an outbox row, in the same transaction as the database change that queues it, and SHALL be delivered only by the background worker, which runs for the lifetime of `kohaku serve`.
- No request handler or CLI command SHALL open an SMTP connection or wait for a delivery result, whether the SMTP server is reachable, slow or unreachable. Queued rows SHALL survive a restart and be delivered after it, subject to their deadlines.
- A row SHALL have security priority if it warns a user about their account's security (account lockout, password changed, invite accepted), else normal. The worker SHALL make at most one delivery attempt at a time, picking a due security row before any due normal row, then the earliest next-attempt time, then the earliest queue time. A row not yet due SHALL NOT delay a due row.
- A failure or panic while building or delivering one row SHALL count as a failed attempt for that row only and SHALL NOT stop the worker or keep other due rows from being attempted.

#### Scenario: Stalled SMTP server does not hold requests
- **WHEN** an attacker sends many mail-triggering requests while the SMTP server is unreachable, or accepts TCP connections but never replies
- **THEN** each request is answered without an SMTP connection opened for it, its row stays unsent for the worker, and Kohaku never has more than one SMTP connection open

#### Scenario: Rolled-back change queues no mail
- **WHEN** a database change that queues a mail is rolled back
- **THEN** no outbox row from it exists and no mail from it is ever sent

#### Scenario: Public mail flood does not delay security mail
- **WHEN** 50 attacker-queued normal rows (such as OTP mail) are due, one of them panics or is permanently rejected for its text, then a security row is queued while another row waits 2 hours for its retry
- **THEN** the security row is attempted first, only the failing row records a failed attempt, and every other due row, including rows queued later, is delivered without waiting for the 2-hour row

### Requirement: Outbox rows name one validated recipient

Each row SHALL record its kind, exactly one recipient, subject, plain-text body, priority, queue time, expiry time, attempt count, next attempt time and whether it carries a token or one-time code.
- The recipient SHALL be a literal address or an account; the database SHALL refuse a row naming both or neither. An account row SHALL be delivered to the account's address at delivery time, and deleting an account SHALL delete its rows.
- An address SHALL have at most 254 characters, exactly one `@` and no whitespace or control characters, or queuing SHALL fail and store no row.
- A placeholder row, queued so that a request does the same work whether or not an account exists, SHALL be marked as such and deleted by the worker without opening an SMTP connection.

#### Scenario: Header injection through the recipient address is refused
- **WHEN** an attacker-supplied address `victim@example.com` followed by CR LF and `Bcc: list@example.net`, or a 255-character address, is queued
- **THEN** queuing fails, no row is stored and no SMTP transaction takes place

#### Scenario: Account rows follow the account
- **WHEN** a row names account 7 and account 7 is deleted before delivery, and another row names an account that does not exist
- **THEN** the first row is gone with the account without an SMTP connection, and the database refuses the second row

#### Scenario: Placeholder row is never sent
- **WHEN** an attacker triggers mail for an address without an account and the feature queues a placeholder row
- **THEN** the worker deletes the row without opening an SMTP connection

### Requirement: Mail is a single plain-text part

Every email SHALL be exactly one `text/plain; charset=utf-8` part, with no HTML, multipart structure or attachments.
- `From` and the envelope sender SHALL be the configured sender; `To` and the envelope SHALL name only the row's address, without display name; no `Cc`, `Bcc` or `Reply-To` SHALL be set.
- User-supplied text SHALL appear only in the subject and body. Every absolute URL SHALL come from the URL builder (configured hosts only, host-routing capability), never from request headers.
- The one `Subject` header SHALL have every control character (Unicode category Cc), U+2028 and U+2029 removed, and if then longer than 200 characters (Unicode scalar values) SHALL be cut to its first 199 plus `…` (U+2026), never splitting a character or failing.

#### Scenario: CR LF in a subject cannot add headers
- **WHEN** an attacker-supplied subject text is CR LF, `Bcc: list@example.net` and 300 four-byte UTF-8 characters
- **THEN** the message has one `text/plain; charset=utf-8` part, the configured `From`, the row's address as sole `To` and envelope recipient, no `Cc`, `Bcc` or `Reply-To`, and one valid UTF-8 `Subject` of 199 characters plus `…` holding `Bcc: list@example.net` only in its value

### Requirement: User-supplied text in mail is single-line, capped and marked as quoted

User-supplied text is any text other than Kohaku's fixed wording, instance configuration values, admin-set project names, numbers, dates, one-time codes and URLs Kohaku builds (for example report titles and close reasons). Before it is placed in a subject or body it SHALL be transformed in this order:
1. each run of control characters (Unicode category Cc), U+2028 and U+2029 replaced by one space;
2. the bidirectional controls U+061C, U+200E, U+200F, U+202A to U+202E and U+2066 to U+2069 removed;
3. cut, if longer than 200 characters, to its first 199 plus `…`, never splitting a character;
4. quoted: in a subject, enclosed in `"` with every `"` inside replaced by `'`; in a body, alone on a line beginning with `> `.

#### Scenario: Line breaks and markup cannot forge mail layout
- **WHEN** an attacker-supplied title `<a href="https://evil.example">Crash</a>` LF LF `Your password expired. Log in at https://evil.example` is put in a body
- **THEN** the body has the single literal line `> <a href="https://evil.example">Crash</a> Your password expired. Log in at https://evil.example`, no other line holds any part of it, and there is no HTML part

#### Scenario: Quoted text cannot close its own quotation
- **WHEN** an attacker-supplied text `x" is fixed. Urgent: verify your account "` with a U+202E inside is put in a subject, and a 500-character text in a body
- **THEN** the subject contains `"x' is fixed. Urgent: verify your account '"` without U+202E, and the body line is `> `, the text's first 199 characters and `…`

### Requirement: SMTP delivery requires verified TLS

The worker SHALL deliver mail only over TLS 1.2 or 1.3, as implicit TLS from the first byte or STARTTLS completed before any authentication or message data, per the configured mode.
- The certificate SHALL chain to the public CA roots compiled into the binary, never the operating system's or image's bundle, and SHALL match the configured SMTP host name. No setting SHALL disable TLS, allow plaintext fallback or skip verification.
- If STARTTLS is not offered or fails, the handshake fails or the certificate does not verify, the attempt SHALL end before credentials, sender, recipient or message data are sent and SHALL count as failed.

#### Scenario: STARTTLS stripping
- **WHEN** an attacker on the network path strips STARTTLS from the EHLO reply, or presents a self-signed, expired or other-host certificate, or the server offers only TLS 1.1 or older or no TLS
- **THEN** Kohaku sends no AUTH, MAIL FROM, RCPT TO or DATA and no byte of the password, closes the connection, and the row is retried on the schedule as a failed attempt

#### Scenario: Verified server
- **WHEN** the SMTP server presents, over TLS 1.2 or 1.3, a certificate for the configured host name that chains to a compiled-in root
- **THEN** the message is delivered

### Requirement: Development builds print mail instead of sending it

A `dev` build SHALL NOT open an SMTP connection. Its worker SHALL write each message it would deliver to stdout between a start and an end marker line: envelope sender and recipient, every header as it would be sent, and the body as readable text (not transfer-encoded). It SHALL then record the attempt as accepted; every other rule of this capability applies unchanged. A build without `dev` SHALL contain no code that prints mail.

#### Scenario: Developer reads a one-time code in the terminal
- **WHEN** a `dev` build's worker picks a due token-bearing row whose body holds a non-ASCII word and a code
- **THEN** stdout shows between the markers the configured `From`, the row's address as `To`, the `Subject`, `Content-Type: text/plain; charset=utf-8` and the body with that word and code as written, no SMTP connection is opened, and the row is deleted as sent

### Requirement: Failed attempts are retried on a fixed schedule

A new row SHALL be due at once and SHALL count as sent only when the server accepts its message data with a 2xx reply; any other outcome (connection, TLS or authentication failure, 4xx or 5xx reply, timeout, interruption) SHALL leave it unsent.
- After the 1st, 2nd, 3rd and 4th failure the next attempt SHALL start 1 min, 5 min, 30 min and 2 h later, then 2 h after each failure, measured from the end of the failed attempt. An attempt unfinished 60 s after it started SHALL be abandoned as failed. Attempt count and next attempt time SHALL be stored with the row, so a restart neither resets nor skips the schedule.
- Each row's expiry SHALL be no later than the token or code it carries (OTP mail: its 10-minute code); its deadline SHALL be the earlier of queue time plus 24 h and that expiry. No attempt SHALL start at or after the deadline.
- A row SHALL be given up when a failure's next attempt would start at or after the deadline, or when the deadline passes undelivered, including while `kohaku serve` is down; the latter SHALL open no SMTP connection.

#### Scenario: Full retry timeline
- **WHEN** every attempt fails at once, for one row expiring 48 hours and another 10 minutes after queuing
- **THEN** the first is attempted at 0, 1, 6 and 36 min, 2 h 36 min and every 2 h to the 15th at 22 h 36 min, and given up after it; the second at 0, 1 and 6 min, and given up after the third

#### Scenario: Restart keeps the schedule
- **WHEN** `kohaku serve` restarts 10 minutes after row A's third failure and 15 minutes after row B, with a 10-minute expiry, was queued
- **THEN** A's fourth attempt starts 30 minutes after its third ended, and B is given up without an SMTP connection

#### Scenario: Acceptance defines sent
- **WHEN** an attacker on the network path keeps the SMTP connection silent, or it drops after the message data but before the reply, or `kohaku serve` shuts down during the attempt
- **THEN** the row is not recorded as sent (a silent attempt is abandoned at 60 s and the worker moves on) and is retried on the schedule, after the next start if shut down, until its deadline

### Requirement: Token-bearing rows are deleted when sent or given up

A row carrying a token or one-time code (invite and password-reset links, OTP codes) SHALL be flagged token-bearing when queued and SHALL be deleted in the same transaction that records it as sent or given up; giving it up SHALL, in that transaction, stop its token or code from being accepted.
- Every other sent or given-up row SHALL be deleted by the next retention run (data-storage capability).
- A deleted row SHALL never be attempted, and a row deleted during its attempt SHALL NOT be recreated by the outcome or attempted again. Deleting a report's or project's rows is added by the changes that create those tables.

#### Scenario: Delivered token mail leaves no copy
- **WHEN** a token-bearing row is delivered or given up, and an attacker later obtains a `kohaku backup` file
- **THEN** the row is gone once the outcome is recorded, neither the outbox nor the backup holds the token, and a given-up token or code is rejected

#### Scenario: Deleted row stays deleted
- **WHEN** an unsent, due row is deleted before the worker picks it or during its attempt
- **THEN** no row with its id exists afterwards, whatever the outcome, and no further attempt is made

### Requirement: SMTP credentials and mail data stay out of logs and errors

The SMTP password (`KOHAKU_SMTP_PASSWORD`) SHALL be sent only to the configured SMTP server, only inside a verified TLS session and only for SMTP authentication.
- It SHALL NOT appear in any log line, error or panic message, CLI output, HTTP response, database row, backup or audit entry, including on rejected authentication, connection or TLS errors and debug formatting of the configuration.
- Worker log entries SHALL contain only row id, kind, priority, attempt number, outcome, failure class and SMTP reply code: no SMTP username, recipient address, subject, body, token, code or reply text.

#### Scenario: Rejected credentials and echoed addresses
- **WHEN** the server answers AUTH with `535 5.7.8 Authentication credentials invalid`, or a recipient with `550 5.1.1 <victim@example.com>: Recipient address rejected`, or startup fails on an invalid SMTP setting
- **THEN** no log line or error holds the password; each failed attempt is logged with its row id, attempt number, failure class (authentication or rejection) and code only, and nothing from it holds the username, `victim@example.com` or other reply text
