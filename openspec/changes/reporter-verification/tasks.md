# Tasks

- [ ] 1.1 Add `migrations/0008_reporter_verification.sql` (D1); verify the §12 migration test.
- [ ] 1.2 Add the keys, codes and the `otp` class (D2; reporter-verification: Verifying an
  address; request-limits: OTP rate-limit class); verify "Attacker guesses a code" and
  "Attacker mail-bombs an address".
- [ ] 1.3 Add the verification pages and API, the token and its single use (D3); verify
  "Token reused or moved" and report-submission's order with the token.
- [ ] 1.4 Add contacts and status mail (D4; reporter-verification: Reporter contacts, Status
  mail to reporters; mail-outbox); verify "Reporter follows their report", the quoted title
  and "implemented" for features.
- [ ] 1.5 Add unsubscribe (D5); verify "Mail scanner and one-click".
- [ ] 1.6 Add erasure and the retention steps; verify "Erasure request" and "Contact purged
  after the report is done".
- [ ] 1.7 Extend `tests/logging.rs` with addresses and codes; README: email verification,
  erasure and backups; `docs/api.md`: OTP endpoints.
- [ ] 1.8 Final check: the before-finishing commands; a dev-build run showing code and status
  mails.
