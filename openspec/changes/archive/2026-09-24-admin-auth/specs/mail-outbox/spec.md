# Spec Delta

## MODIFIED Requirements

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
