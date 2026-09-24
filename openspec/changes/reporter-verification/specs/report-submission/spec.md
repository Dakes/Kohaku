# Spec Delta

## MODIFIED Requirements

### Requirement: Order of checks

A submission SHALL be checked in this order, the first failure answering, and SHALL read its
body only after steps 1–3:

1. Fetch-Metadata/Origin (http-security).
2. `submit` rate-limit class (request-limits).
3. Project lookup, then the pending caps from a read connection as an early rejection.
4. Body read under the 64 KiB cap.
5. Honeypot, then proof of work (consumed), then the verified-email token when the project
   requires one (reporter-verification), then field validation, then Markdown checks.
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
