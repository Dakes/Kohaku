# Tasks

## 1. Data

- [ ] 1.1 Add `migrations/0006_moderation.sql` (D1); verify the §12 migration test and that
  the views expose only public statuses and columns.
- [ ] 1.2 Add the public-read module and its source-scan rule (D2); verify a planted query on
  `reports` from a public handler fails the scan.

## 2. Moderation

- [ ] 2.1 Add transitions with audit (D3; moderation: Status transitions); verify every row of
  the table, every refused pair, "Stale or forged transitions", close-reason rules and
  unhiding an unapproved report.
- [ ] 2.2 Add the maintainer list and detail pages with inert links for pending reports (D5;
  moderation: Maintainer pages and actions); verify "Maintainer reaches into another project"
  and the `/admin` counts.
- [ ] 2.3 Add edits, kind changes and notes with audit; verify each audit action and the kind
  rule.
- [ ] 2.4 Add bulk actions (D4); verify "Bulk reject by source" and the 1000-row cap.
- [ ] 2.5 Add admin-only permanent deletion; verify "Maintainer cannot hard-delete".
- [ ] 2.6 Add the spam purge (D7; data-storage: Retention job); verify "Spam is purged".

## 3. Public

- [ ] 3.1 Add the public list and detail pages with tabs, kind filter and labels (D6;
  moderation: Public pages and read API); verify labels with and without feature requests,
  "Fixed" vs "Implemented", and "Paging while reports change".
- [ ] 3.2 Add the read API with CORS; verify "Attacker sends crafted list parameters" and the
  JSON fields.
- [ ] 3.3 Verify "Attacker probes private reports" on pages and API.

## 4. Documentation and check

- [ ] 4.1 Extend `docs/api.md` with the read API; README: moderation.
- [ ] 4.2 Final check: the before-finishing commands, the smoke test (an approved report is
  listed on the project host), a real-browser pass over the maintainer pages.
