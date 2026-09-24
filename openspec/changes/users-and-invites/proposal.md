# Proposal

## Why

`admin-auth` can check who someone is, but no account can exist yet and nothing limits a
maintainer to their projects. This change creates the first admin from the command line,
brings in maintainers by invitation with per-project grants, and adds the admin's tools for
lost second factors, disabled accounts and a lost instance secret (design §8, §15 item 4).

## What Changes

- `kohaku admin create --email` prints a 1-hour setup link for the one admin account.
- Setup links (`/admin/setup/{token}`): set the password and, where required, enroll TOTP with
  a QR code; also made by `admin reset-2fa`, `admin rekey` and "Reset 2FA" on the users page.
- Invitations mailed with a 48-hour link and project grants; pending list with revoke;
  acceptance creates the maintainer and mails the admin.
- `project_members` grants and per-project access: `/admin/p/{slug}/…` answers 404 without a
  grant.
- Disabling and re-enabling maintainers.
- "Require 2FA for maintainers".
- `kohaku admin rekey` after a lost `KOHAKU_SECRET`.
- `/admin/users` for the admin.

## Capabilities

### New Capabilities

- `users-and-invites`: setup links, the first admin, invitations, grants and project access,
  disabling, 2FA reset and requirement, rekey, the users page.

### Modified Capabilities

- `admin-auth`: maintainers can be required to use 2FA at login.
- `audit-log`: the actions of this change.
- `operations`: `admin create`, `admin reset-2fa`, `admin rekey`.
- `configuration`: their settings; `admin rekey` skips the keycheck.
- `data-storage`: `admin rekey` holds the instance lock; retention covers setup and
  invitation tokens.
- `request-limits`: setup and invitation POSTs use the `login` class.

## Non-goals

- Deleting accounts (disable instead; audit entries keep their ids), changing an account's
  address or role, more than one admin, self-registration.
- Maintainer notification settings per project (`notifications`).
- Grants with finer permissions than "works on this project".

## Security considerations

- **Attacker: intercepts a setup or invitation link.** Setup links are never mailed and live 1
  hour; invitations live 48 hours, are single use, revocable, and the admin gets a mail when
  one is accepted. Both are consumed by one conditional write.
- **Attacker: a maintainer probing other projects.** One lookup binds project and grant;
  every miss is the same 404 as a nonexistent project.
- **Attacker: a disabled or ex-maintainer.** Disabling ends sessions, voids links and deletes
  queued mail at once.
- **Attacker: lost or stolen second factor.** Resetting 2FA ends sessions and devices at
  once; only the admin or the CLI can do it.
- **Operator: lost secret.** `admin rekey` works only with the server stopped and container
  access, which already means full control.

No new dependencies.

## Impact

Migration 4 (`project_members`, `invite_grants`, `meta.require_2fa_maintainers`). Users page,
setup and invitation templates, two mail kinds (invitation, invitation accepted), three CLI
commands, the `Project` access level with grant binding.
