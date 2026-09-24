# Proposal

## Why

Some projects want a stronger gate than proof of work, and reporters want to know when their
report moves. A verified address does both, and must be kept only as long as it serves the
reporter (design §6 Email OTP, §9, §10 Erasure, §15 item 8).

## What Changes

- Email one-time codes (6 digits, 10 minutes, 5 attempts, HMACs only) on projects with
  `require_email`; a verified-email token (30 minutes, single use) for the submission.
- `otp` rate class; per-address limit 3 per hour; the shared public mail budget; `email_code`
  proof of work.
- Contacts per report; status mail (approved, in progress, fixed/implemented, closed) with
  one-click unsubscribe; contacts purged 30 days after the report is done; erasure by the
  admin.
- Outbox rows gain a report reference; two mail headers allowed.

## Capabilities

### New Capabilities

- `reporter-verification`: verification, contacts, status mail, unsubscribe, erasure.

### Modified Capabilities

- `mail-outbox`: `List-Unsubscribe` headers.
- `report-submission`: the token check in the order of checks.
- `request-limits`: the `otp` class.
- `audit-log`: `report.erase_contact`.
- `data-storage`: retention of contacts and codes.

## Non-goals

- Verification on projects without `require_email`, reporter accounts, reply by mail, mail
  about notes.

## Security considerations

- **Attacker: code guessing.** 5 attempts per id, consumed before comparing, 10 minutes,
  `otp` bucket.
- **Attacker: mail-bombing a victim.** 3 codes per hour per normalized address, shared mail
  budget, proof of work per send.
- **Attacker: phishing through report titles in reporter mail.** Titles and close reasons are
  single-line, capped and quoted; links come only from the URL builder.
- **Attacker with a backup.** Codes and addresses in verification are HMACs under per-boot
  keys; contacts live only while useful and are deleted 30 days after the report is done.

No new dependencies.

## Impact

Migration 8 (`email_codes`, `report_contacts`, `outbox.report_id`). Verification pages,
unsubscribe pages, three mail kinds, two per-boot keys.
