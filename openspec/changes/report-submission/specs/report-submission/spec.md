# Spec Delta

## Purpose

How the public files bug reports and feature requests without an account: the form and JSON
API, proof of work, honeypot, rate limits and pending caps, field rules, and the pending
report that results (design §6 Submission, Proof of work, Public reads and JSON API).

## ADDED Requirements

### Requirement: Reports

A report SHALL belong to one project and have a per-project number (from 1, never reused
within the project, even after deletion), a kind (`bug` or `feature`), a title, a Markdown
body, a status (`pending`, `open`, `in_progress`, `fixed`, `closed`, `spam` or `hidden`,
moderation defines the transitions), creation and status-change times, and, while pending, a
source key. A submission SHALL create exactly one report with status `pending`, and nothing
about a pending report SHALL be public (moderation defines visibility). A report SHALL store
no email address, IP address or network prefix; the source key is the only value derived
from the submitter's network.

#### Scenario: Numbers survive deletion
- **WHEN** reports 1 to 3 of project demo exist, report 3 is deleted, and a new report is submitted
- **THEN** the new report is number 4

### Requirement: Submission routes

Relative to a project's base (`/p/{slug}` on the main host, `/` on its custom domain):

- `GET /new` SHALL render the form: title, body, the kind choice when the project accepts
  feature requests, a hidden honeypot field, an embedded proof-of-work challenge, the
  project's privacy notice and security contact when set, and a notice that the form needs
  JavaScript for the proof of work.
- `POST /new` (form-urlencoded) SHALL answer `303` to `received` on success, or re-render the
  form with the reason and a fresh challenge, the entered title and body kept.
- `GET /received` SHALL render a fixed page saying the report will be public after review.
- `GET /api/v1/challenge?purpose=submit` SHALL return `{"challenge": "…"}` for the project.
- `POST /api/v1/reports` with `Content-Type: application/json` and
  `{"title", "body", "kind"?, "pow"}` SHALL answer `202` with `{}` on success; errors SHALL be
  the HTTP status with `{"error": "<code>", "message": "<text>"}`, plus a fresh
  `"challenge"` when the retry needs one. `kind` defaults to `bug`.

The JSON routes SHALL be exempt from the Fetch-Metadata/Origin rule for requests carrying
neither header (native clients), and nothing else; GET API responses SHALL carry
`Access-Control-Allow-Origin: *` and no other CORS header, and no response SHALL allow
credentials or answer a preflight with CORS headers. A project that does not exist SHALL
answer 404 on every route, the same as any unknown path.

#### Scenario: Attacker's page posts cross-site
- **WHEN** an attacker's page submits the HTML form cross-site, and a script on another origin posts JSON with `Origin: https://evil.example`
- **THEN** both get 403 and no report is created, while a native client posting JSON with neither header is processed

#### Scenario: Preflight gets no CORS
- **WHEN** a browser sends `OPTIONS /api/v1/reports` with `Access-Control-Request-Method: POST` from another origin
- **THEN** the response carries no `Access-Control-Allow-*` header

### Requirement: Order of checks

A submission SHALL be checked in this order, the first failure answering, and SHALL read its
body only after steps 1–3:

1. Fetch-Metadata/Origin (http-security).
2. `submit` rate-limit class (request-limits).
3. Project lookup, then the pending caps from a read connection as an early rejection.
4. Body read under the 64 KiB cap.
5. Honeypot, then proof of work (consumed), then field validation, then Markdown checks.
6. One write that takes the next number and inserts the pending report only if both pending
   caps still hold, decided by rows affected, holding a public-write permit (request-limits).

A filled honeypot SHALL get exactly the success response and create nothing. Every check
that fails before step 6 SHALL create nothing and consume no report number.

#### Scenario: Honeypot hit looks like success
- **WHEN** a bot fills the hidden field and posts the form with a valid proof of work
- **THEN** it gets `303` to `received`, no report is created and no number is consumed

#### Scenario: Rejected early without reading the body
- **WHEN** an attacker whose network already has 5 pending reports in project demo posts a 64 KiB body slowly
- **THEN** it is answered before the body is read

### Requirement: Proof of work

Every submission SHALL carry a solved challenge. A challenge SHALL be issued by Kohaku,
authenticated with a per-boot key (label `kohaku/pow`), and bind its purpose (`submit`,
and later `email_code` and `plus_one`), the project, a 128-bit random nonce, the difficulty
and an expiry 10 minutes after issue. A solution is a counter such that SHA-256 of the
challenge followed by the counter has at least the purpose's number of leading zero bits.
The server SHALL check the MAC, purpose, project, expiry and the difficulty constant of the
purpose (never the one in the challenge), then consume the nonce atomically before any
further work; a nonce SHALL be accepted at most once. The consumed-nonce set SHALL be
memory-only, pruned at expiry, and capped at 200 000 entries shared by all purposes; when
full, submissions SHALL get 503. The `submit` difficulty SHALL be a constant set so that
95 % of challenges take at most 4 seconds on a low-end phone, measured once and recorded
with the constant. The form's script SHALL solve the challenge in a web worker served from
`/static/` and submit it in a hidden field, with no inline script and no change to the
Content Security Policy.

#### Scenario: Attacker replays, forges or reuses challenges
- **WHEN** an attacker submits one solution twice, a challenge for another project, one with its difficulty lowered, one with a flipped MAC bit, one 11 minutes old, and one issued before a server restart
- **THEN** only the first submission of the first solution is accepted; each other gets 400 with a fresh challenge and creates nothing

#### Scenario: Used-set flood
- **WHEN** the consumed-nonce set holds 200 000 unexpired entries and another valid solution arrives
- **THEN** it gets 503 and no report is created

### Requirement: Field rules

- Title: 1–200 Unicode scalar values after removing surrounding whitespace, single line as
  the projects capability defines it.
- Body: Markdown source of at most 20 480 bytes of UTF-8 that passes the markdown
  capability's checks; empty allowed.
- Kind: `bug`, or `feature` only while the project accepts feature requests; the HTML form
  SHALL require an explicit choice when it offers one.

A violation SHALL get 422 naming the field and create nothing.

#### Scenario: Attacker hides or overflows text
- **WHEN** an attacker submits a title with U+202E or a line feed, a 201-character title, a 20 481-byte body, or kind `feature` to a project without feature requests
- **THEN** each gets 422 naming the field and no report is created

### Requirement: Pending caps

Each project SHALL accept at most 5 pending reports from one source (the IPv4 /32 or IPv6 /48
of the resolved client address) and at most 500 pending reports in total; both are constants.
The source key SHALL be HMAC-SHA256 of the prefix under a key derived from the instance secret
with label `kohaku/source`, stored only on pending reports and cleared when a report leaves
`pending`. At the per-source cap the response SHALL say too many reports from this network
are waiting for review (429); at the project cap the form SHALL say the project is not
accepting reports right now and posts SHALL get 503, until moderation brings the count below
500. Both caps SHALL hold under concurrent submissions.

#### Scenario: Parallel flood from one network
- **WHEN** an attacker with 3 pending reports in project demo sends 10 valid submissions in parallel from one /48
- **THEN** exactly 2 are created and the others get 429

#### Scenario: Leaked backup reveals no networks
- **WHEN** an attacker holding a backup and a candidate IP address tries to test whether it submitted a pending report
- **THEN** without `KOHAKU_SECRET` no stored value lets them check it
