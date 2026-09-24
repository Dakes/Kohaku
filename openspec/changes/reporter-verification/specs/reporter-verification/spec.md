# Spec Delta

## Purpose

Email verification for reporters on projects that require it, and what a verified address is
used for: mail about the report's progress, one-click unsubscribe, deletion after the report
is done, and erasure on request (design §6 Email OTP, §9 Verified reporters, §10 Erasure).

## ADDED Requirements

### Requirement: Verifying an address

On a project with `require_email` on, the report form SHALL first ask for an email address
and, after a verified code, carry a verified-email token; on other projects the form SHALL
ask for no address and no address SHALL be accepted. Relative to the project base:

- `POST /verify/send` with an address and a solved `email_code` proof of work SHALL answer
  with the code page (a random 128-bit verification id in a hidden field) whatever the address,
  after the `otp` rate-limit token, the per-address limit of 3 codes per hour (the normalized
  address, request-limits), and the public mail budget. It SHALL queue a token-bearing mail
  whose body is only the 6-digit code and fixed wording, expiring with the code after 10
  minutes. Only HMACs of the address and the code under a per-boot key SHALL be stored.
- `POST /verify/check` with the verification id, the address and the code SHALL take an `otp`
  rate-limit token and consume one of 5 attempts for that id in one conditional write before
  comparing; a match SHALL delete the code and render the report form with a verified-email
  token valid 30 minutes for that project and address.
- The submission SHALL accept the token once (the verification id enters the proof-of-work
  used set until the token expires, released if the insert fails) and take the reporter's
  address only from it.

Pages never call the JSON API; the API has `POST /api/v1/otp/send` and `/api/v1/otp/verify`
with the same rules, and `email_token` in `POST /api/v1/reports`.

#### Scenario: Attacker guesses a code
- **WHEN** an attacker holding a verification id tries 6 codes, and another tries the right code with a different address
- **THEN** the 6th try is refused even if right, and the address mismatch fails, consuming an attempt

#### Scenario: Attacker mail-bombs an address
- **WHEN** an attacker requests codes for `victim@example.org` and `Victim+x@example.org` from 5 networks within an hour
- **THEN** at most 3 codes are queued for that mailbox and the others get 429

#### Scenario: Token reused or moved
- **WHEN** a verified-email token is used for two submissions, for another project, or after 30 minutes
- **THEN** only the first submission to its own project within 30 minutes succeeds

### Requirement: Reporter contacts

A report submitted with a verified address SHALL store it in a contact record with its own
unsubscribe token (256 bits, stored as SHA-256) and nowhere else. The record SHALL be deleted
30 days after the report enters `fixed`, `closed`, `hidden` or `spam`, when the reporter
unsubscribes, when the admin erases it, and with the report. No page or API response SHALL
show the address; maintainers see only whether a report has a contact.

#### Scenario: Contact purged after the report is done
- **WHEN** a report with a contact is fixed and 30 days pass
- **THEN** the address is gone from the database, and a later status change queues no mail

### Requirement: Status mail to reporters

When a report with a contact becomes `open` (approved), `in_progress`, `fixed` or `closed`,
Kohaku SHALL queue, in the transaction of the transition, one mail to the contact stating the
project name, the report number, its title quoted as mail-outbox requires, the new status
(for a feature request, "implemented" instead of "fixed"), the close reason when closed,
quoted the same way, and the report's public URL. The mail SHALL carry
`List-Unsubscribe: <https://{main host}/unsubscribe/{token}>` and
`List-Unsubscribe-Post: List-Unsubscribe=One-Click`, and the body SHALL repeat the link.

#### Scenario: Reporter follows their report
- **WHEN** a verified reporter's report is approved, set in progress and fixed
- **THEN** three mails are queued in the transitions' transactions, each with the report link and both unsubscribe headers

### Requirement: Unsubscribe

`GET /unsubscribe/{token}` on the main host SHALL only render a confirmation page (the same
"link invalid" page for any unknown token). `POST /unsubscribe/{token}` SHALL delete the
contact and its unsent mail in one conditional write; it SHALL be exempt from the
Fetch-Metadata/Origin rule for requests carrying neither header (RFC 8058 one-click senders)
and accept a form-urlencoded or multipart body of at most 64 KiB, which it does not parse; it
SHALL take a `read` rate-limit token.

#### Scenario: Mail scanner and one-click
- **WHEN** a mail scanner fetches the link with GET, then the mail provider posts `List-Unsubscribe=One-Click` with neither `Origin` nor `Sec-Fetch-Site`
- **THEN** the GET changes nothing and the POST deletes the contact

### Requirement: Erasure

The admin SHALL erase a report's contact from the report page: in one transaction the contact
and the report's unsent mail are deleted and `report.erase_contact` is audited, followed by a
WAL checkpoint. The README SHALL state that earlier backups still hold erased data.

#### Scenario: Erasure request
- **WHEN** the admin erases the contact of a report with a queued status mail
- **THEN** the address and the mail are gone from the database and its WAL after the checkpoint
