# Tasks

## 1. Schema and access

- [ ] 1.1 Add `migrations/0004_users_and_invites.sql` (D1); verify the §12 migration test with
  sample rows.
- [ ] 1.2 Add the `Project(Cap)` access level and move project settings to it (D3); verify
  "Maintainer probes an ungranted project", "Grant revoked mid-session" with a synthetic
  `Moderate` route, and matrix rows.

## 2. Setup links and the first admin

- [ ] 2.1 Add setup tokens and pages (D2; users-and-invites: Setup links); verify "Link holder
  races or replays" and "Admin must enroll during setup", QR code and recovery codes shown
  once, and `user.setup`.
- [ ] 2.2 Add `kohaku admin create` (users-and-invites: First admin from the command line;
  operations; configuration); verify "Bootstrapping twice" and the one-admin index.
- [ ] 2.3 Add `kohaku admin reset-2fa` and the admin's "Reset 2FA" (D7; users-and-invites:
  Resetting a lost second factor); verify "Maintainer lost their phone" and `user.totp_reset`.

## 3. Invitations and grants

- [ ] 3.1 Add invitation creation, the pending list and revoke with their mail kind (D4;
  users-and-invites: Invitations); verify 422 for an existing account, replacement, the mail
  body, `invite.create` and `invite.revoke`.
- [ ] 3.2 Add acceptance (D4); verify "Invitation raced against revocation", "Granted project
  deleted before acceptance", "Attacker holds an old or revoked link", the admin mail and
  `invite.accept`.
- [ ] 3.3 Add grant editing and `/admin` listing granted projects (users-and-invites: Grants
  and project access); verify `user.grants`.

## 4. Account administration

- [ ] 4.1 Add disable and enable (D5; users-and-invites: Disabling accounts); verify "Disabled
  maintainer with a live session and a reset link".
- [ ] 4.2 Add the 2FA requirement switch (users-and-invites: Requiring 2FA for maintainers;
  admin-auth: Login); verify "Requirement switched on" and that disabling TOTP is refused
  while on.
- [ ] 4.3 Add the users page (D7; users-and-invites: Users page); verify "Maintainer probes
  user administration".

## 5. Rekey, limits, retention and documentation

- [ ] 5.1 Add `kohaku admin rekey` (D6; users-and-invites: Recovering from a lost instance
  secret; configuration: Database keycheck; data-storage: Instance lock); verify both
  scenarios and `instance.rekey`.
- [ ] 5.2 Put setup and invitation POSTs in the `login` class and their tokens in retention;
  verify matrix rows and data-storage "Expired sign-in state is purged" with both purposes.
- [ ] 5.3 Extend the smoke test: `admin create` prints a link that renders the setup page.
- [ ] 5.4 README: first start (`admin create`), inviting maintainers, lost 2FA, lost secret
  (`admin rekey`).
- [ ] 5.5 Final check: the before-finishing commands, the smoke test, and a real-browser run
  of setup, invitation and login recorded in the PR.
