# Tasks

## 1. Data and Markdown

- [ ] 1.1 Add `migrations/0005_reports.sql` (D1); verify the §12 migration test and "Numbers
  survive deletion".
- [ ] 1.2 Add `pulldown-cmark` and `ammonia` (dependencies, deny.toml) and the Markdown module
  (D5; markdown: Accepted Markdown, Rendering is sanitized); verify both scenarios with a
  checked-in XSS corpus and the inert-link mode.

## 2. Proof of work and limits

- [ ] 2.1 Add `Purpose::Pow`, challenges, verification and the used set (D3; report-submission:
  Proof of work); verify "Attacker replays, forges or reuses challenges" and "Used-set flood".
- [ ] 2.2 Add `static/pow-worker.js` and `static/submit.js` (D4); verify in a browser that
  the form solves a challenge under the release CSP without violations, and record the
  low-end-phone measurement next to the constant.
- [ ] 2.3 Add the `submit` class (request-limits: Submit rate-limit class); verify its
  scenario and matrix rows.
- [ ] 2.4 Add `Purpose::Source` and the source key; clear it in `admin rekey`
  (users-and-invites: Recovering from a lost instance secret); verify "Leaked backup reveals
  no networks".

## 3. Submission

- [ ] 3.1 Add the form, `received` page and challenge endpoint on both hosts
  (report-submission: Submission routes); verify privacy notice, security contact and kind
  choice rendering and 404 for unknown projects.
- [ ] 3.2 Add the check order and insert (D2; report-submission: Order of checks, Field
  rules, Pending caps); verify "Honeypot hit looks like success", "Rejected early without
  reading the body", "Attacker hides or overflows text" and "Parallel flood from one
  network".
- [ ] 3.3 Add `POST /api/v1/reports` with JSON errors and CORS (D8); verify "Attacker's page
  posts cross-site", "Preflight gets no CORS" and each error code.
- [ ] 3.4 Extend `tests/logging.rs` with submissions carrying marker text; verify none reaches
  the log.

## 4. Documentation and check

- [ ] 4.0 Add a cargo-fuzz target for the Markdown pipeline (`-timeout=2 -max_len=20480`,
  design §12), run manually on nightly outside CI (the pinned toolchain stays stable); verify
  a 10-minute run finds no panic or sanitizer escape and record it in the PR.

- [ ] 4.1 Document the API (routes, PoW algorithm with an example client in shell or Python)
  in `docs/api.md`, and the form in the README.
- [ ] 4.2 Final check: the before-finishing commands and the smoke test (a submission through
  Caddy on a project host).
