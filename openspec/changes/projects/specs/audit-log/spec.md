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
  creates a project, changes any of its fields or deletes it (target type `project`).

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
