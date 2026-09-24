# Proposal

## Why

Every report, form, list and grant belongs to a project, and a project can live on its own
domain. Projects, their switches and custom domains come before reports so that every later
change finds its project, its host and its flags in place (design §4, §6, §15 item 3).

## What Changes

- `projects` table: slug, name, optional custom domain, the switches `screenshots_enabled`,
  `features_enabled`, `require_email`, `plus_one_enabled`, privacy notice, security contact,
  report numbering.
- Admin pages to list, create, edit and delete projects; `/admin` lists projects for the admin.
- The `Admin` access level for routes (signed-in admin only; 404 for maintainers).
- Custom domains: host map entries per project, rebuilt when a change commits and within 2 s
  after changes by another process; tls-ask answers 200 for them.
- `308` from `/p/{slug}/…` on the main host to the project's domain (not for the API or
  writes).
- `kohaku project create <slug> --name <name> [--host <host>]`.
- Plan change: `projects` now comes before `users-and-invites` (grants reference projects).

## Capabilities

### New Capabilities

- `projects`: project fields and switches, administration, custom domains, canonical
  redirect, the `project create` command.

### Modified Capabilities

- `host-routing`: the host map holds project hosts; tls-ask answers for them; the main
  router serves project administration and the redirect.
- `audit-log`: `project.create`, `project.update`, `project.delete`.
- `operations`: `kohaku project create`.
- `configuration`: settings `project create` needs.

## Non-goals

- Public project pages, report forms and lists (`report-submission`, `moderation`); until
  then a project host serves only static assets and `/p/{slug}` without a domain is 404.
- Maintainer grants and per-project access (`users-and-invites`).
- Renaming a slug, transferring reports between projects, archiving projects.
- What the switches do: each takes effect in the change that reads it.

## Security considerations

- **Attacker: points DNS at the server to get certificates.** Caddy asks tls-ask, which says
  yes only for a configured project host; the main host has its own site block.
- **Attacker: forged `Host` or forwarded headers on the redirect.** The location is built only
  from the stored host; unknown hosts keep getting 421.
- **Attacker: a signed-in maintainer.** Project administration is admin-only and answers 404
  like a nonexistent page.
- **Operator mistake: deleting a project.** The slug must be typed to confirm; the deletion is
  audited. It cannot be undone except by restoring a backup.
- **Stale host after a domain move.** The old host is removed from the map on commit, so it
  gets 421 and no new certificate.

No new dependencies.

## Impact

Migration 3 (`projects`). Admin templates for the project list and settings. The host-map
watcher connection (design §3 DB access) starts here. CLI grammar and smoke test extended
(`project create --host`, the project host gets a certificate, `/p/{slug}` answers 308).
