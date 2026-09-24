# Spec Delta

## MODIFIED Requirements

### Requirement: Audited actions

Every state-changing admin action (by a signed-in admin or maintainer) and CLI command
SHALL record audit entries whose action identifier, target type and ids the change adding
it defines. The actions so far, each with target type `user` and the account's id unless
stated otherwise:

- `instance.restore`: `kohaku restore` (target `instance`);
- `user.unlock`: `kohaku admin unlock`;
- `user.reset_link`: `kohaku admin reset-password`;
- `user.password_change`: a signed-in user changes their password;
- `user.password_reset`: a password set through a reset link, the account as actor;
- `user.totp_enable`, `user.totp_disable`, `user.recovery_codes`: enrolling or re-enrolling
  TOTP, disabling it, regenerating recovery codes;
- `user.sessions_end`: a user ends their other sessions;
- `project.create`, `project.update`, `project.delete`: the admin or `kohaku project create`
  creates a project, changes any of its fields or deletes it (target type `project`);
- `user.create`: `kohaku admin create`;
- `user.setup`: a setup link is used, the account as actor;
- `user.totp_reset`: `kohaku admin reset-2fa` or the admin's "Reset 2FA";
- `user.grants`, `user.disable`, `user.enable`: the admin changes a maintainer's grants,
  disables or re-enables the account;
- `invite.create`, `invite.revoke`: the admin invites or revokes (target type `invite`, the
  invitation's id);
- `invite.accept`: an invitation is accepted, the new account as actor and target;
- `instance.require_2fa`: the admin switches "require 2FA for maintainers" (target
  `instance`);
- `instance.rekey`: `kohaku admin rekey` (target `instance`);
- `report.approve`, `report.reject`, `report.status`: approving, rejecting as spam (once per
  report in bulk actions), and every other status transition (target type `report`);
- `report.edit`, `report.kind`, `report.delete`: editing title or body, changing the kind,
  permanent deletion by the admin;
- `note.create`, `note.edit`, `note.delete`: public notes (target type `note`);
- `user.notify`: a user switches mail for a project on or off;
- `report.erase_contact`: the admin erases a report's contact (target type `report`).

Nothing else SHALL, notably:

- an action that ends without changing stored state (refused, invalid or failed);
- routes needing neither a session nor an invite, reset or setup token (public pages,
  report forms, public JSON API, one-click unsubscribe, landing page, internal endpoints,
  login and reset requests);
- signing in and out, failed logins, locks, lockout mail and recording a session's use;
- `kohaku healthcheck`, `backup <file>`, `backup -`, `restore --list`, `serve` startup and
  the server's own work (startup migrations, retention, outbox delivery, DB maintenance).

#### Scenario: Anonymous request flood
- **WHEN** an attacker sends GET and POST requests with arbitrary paths, headers and bodies to every main-host route, `/admin/…`, unknown hosts, `/healthz` and `/.well-known/kohaku/tls-ask`
- **THEN** the number of audit entries is unchanged

#### Scenario: Read-only commands and background work
- **WHEN** an operator runs each read-only command above, `kohaku serve` starts and migrates an existing database, and the outbox worker sends a mail or gives up on one
- **THEN** the number of entries in the live database is unchanged
