# Design

## Context

Builds on `report-submission` (PoW, used set, insert), `moderation` (transitions), the
per-address bucket from `admin-auth`. Design §6 Email OTP and §9 are the source.

## Decisions

**D1. Schema (`migrations/0008_reporter_verification.sql`).** `email_codes(verification_id
BLOB PRIMARY KEY, project_id → projects CASCADE, email_mac, code_mac, attempts, expires_at)`;
`report_contacts(report_id PRIMARY KEY → reports CASCADE, email, unsubscribe_hash UNIQUE)`;
`ALTER TABLE outbox ADD COLUMN report_id INTEGER REFERENCES reports ON DELETE CASCADE`.

**D2. Keys.** Per-boot `kohaku/otp` (code and address MACs) and `kohaku/verified-email`
(token: MAC over project id, verification id, address and expiry, plus those fields,
base64url). Both in §5's inventory already (OTP key, verified-email key).

**D3. Pages.** On a `require_email` project `GET /new` shows the address step first
(address, PoW for `email_code`); `/verify/send` renders the code step; `/verify/check`
renders the report form with the token in a hidden field. All forms post normally
(report-submission D6).

**D4. Status mail.** Queued by the transition function when the target status is one of the
four and a contact exists; kind `reporter.status`, normal priority, 24 h; subject
`[<project name>] Report #<n> "<title>" is <status>` built with the mail-outbox text rules.

**D5. Unsubscribe.** Main-host routes, `read` class, exempt from rule 3 by declaration, body
not parsed; the token is looked up by SHA-256.

## Risks / Trade-offs

- [A restart invalidates outstanding codes and tokens] → Intended (§5): no replay after
  restart; the reporter requests a new code.
- [Reporters on shared networks share the `otp` bucket] → 20 per hour per /64.

## Migration Plan

Migration 8. Rollback before a release: revert.
