# Design

## Context

Builds on `admin-auth` (accounts, `tokens` with `totp_nonce`, sessions, the enrollment page)
and `projects` (`Admin` access, host map). Design §8 Token flows and Access control are the
source; the order swap with `projects` is recorded there (D8.4).

## Goals / Non-Goals

**Goals:** every link is single use by one conditional write; project access is one query
that cannot forget the grant.

**Non-Goals:** see proposal.

## Decisions

**D1. Schema (`migrations/0004_users_and_invites.sql`).** `project_members(user_id →
users CASCADE, project_id → projects CASCADE, PRIMARY KEY (user_id, project_id))` (mail
opt-outs live in `notifications`' own table, since the admin has no grant rows); `invite_grants(token_id → tokens CASCADE, project_id → projects
CASCADE, PRIMARY KEY (token_id, project_id))`; `ALTER TABLE meta ADD COLUMN
require_2fa_maintainers INTEGER NOT NULL DEFAULT 0 CHECK (… IN (0,1))`.

**D2. Token pages.** Setup and invitation share admin-auth's reset mechanics: shape check
on the path parameter, GET renders from a reader, POST consumes with `UPDATE tokens SET
used_at … RETURNING`. The enrollment nonce is drawn when the token is created and stored in
`tokens.totp_nonce`, so GET shows the QR code without writing. A re-rendered form after a
wrong code keeps the token unused.

**D3. Access level `Project(Cap)`.** Route-table access gains `Project(Moderate)` and
`Project(ProjectAdmin)`. The guard runs `SELECT p.id FROM projects p LEFT JOIN
project_members m ON m.project_id = p.id AND m.user_id = :uid WHERE p.slug = :slug AND
(:is_admin OR m.user_id IS NOT NULL)` on a reader and hands handlers only that id;
`ProjectAdmin` needs the admin role. Projects' `/admin/p/{slug}/settings` and delete move to
`Project(ProjectAdmin)` (same 404 behaviour). No route of this change uses `Moderate`; tests
use a synthetic one, `moderation` adds the real ones.

**D4. Invitations.** Creation: one transaction deletes the address's pending invitation,
inserts the token (48 h, nonce), its grants and the mail row (kind `account.invite`,
token-bearing, expiry 48 h; give-up marks the token used). Acceptance: consume, `INSERT INTO
users` (unique email; a conflict rolls back with a message), `INSERT INTO project_members
SELECT … FROM invite_grants JOIN projects`, mail `account.invite_accepted` (security) to the
admin, audit — one transaction.

**D5. Disable.** One transaction: `disabled = 1`, delete sessions, mark unused tokens used,
delete the account's unsent outbox rows through the shared give-up function.

**D6. Rekey.** Holds the lock, opens the database without the keycheck, runs the downgrade
guard, then one transaction: clears `totp_nonce`, `totp_last_step`, recovery codes and
sessions, stores the new keycheck, creates setup tokens for previously enrolled accounts,
audits. Later changes add their derived columns to this transaction (`reports.source_key`).
Output: `<email>\t<link>\n` per account in id order.

**D7. Users page.** `/admin/users` (list, invite form, pending invitations, the switch),
`/admin/users/{id}` (grants, disable/enable, Reset 2FA). `Admin` access, CSRF, no rate class.
"Reset 2FA" is offered for maintainers only; the admin resets their own with the CLI (their
session would end mid-action).

**D8. Deviations from the design.**

1. **Setup always sets a new password** (§8 says it "sets the password and enrolls TOTP"):
   also after `reset-2fa`, since the link holder must prove nothing else.
2. **Invitation and setup pages require TOTP only when the account must use it** (§8 is
   silent for maintainers): optional enrollment stays on the account page.
3. **Invitation-accepted mail names no address** (§9): fixed text and a link to the users
   page.
4. **Order after `projects`** (§15): grants reference projects.
5. **No web "Reset 2FA" for the admin's own account**: CLI only.

## Risks / Trade-offs

- [An invitation mail can wait up to 24 h in the outbox; given up, its token dies] → The
  admin sees it pending and can invite again.
- [Rekey forces every enrolled account through setup] → Documented; only after a lost secret.

## Migration Plan

Migration 4 adds tables and a `meta` column. Rollback before a release: revert.
