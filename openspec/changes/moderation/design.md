# Design

## Context

Builds on `report-submission` (reports table, Markdown) and `users-and-invites` (grants,
`Project(Moderate)`). Design §4 Lifecycle and Visibility, §6 Public reads, §8 Moderation are
the source.

## Goals / Non-Goals

**Goals:** a forgotten filter fails closed; no admin query can cross projects.

**Non-Goals:** see proposal.

## Decisions

**D1. Schema (`migrations/0006_moderation.sql`).** `report_notes(id INTEGER PRIMARY KEY
AUTOINCREMENT, report_id → reports CASCADE, body, created_at, updated_at)`. Views:
`public_reports` (`status IN ('open','in_progress','fixed','closed')`, public columns only)
and `public_notes` (joined through `public_reports`). Indexes: `(project_id, status, number
DESC)`, `(project_id, status, status_changed_at DESC, number DESC)`, both with `kind` for
the filter.

**D2. One public-read module.** `public.rs` is the only code that reads reports for public
routes and the only one allowed to name `public_reports`/`public_notes`; `PublicReport` and
`PublicNote` are the only serialized types. A source-scan test fails when another module
under the public routes names `reports` or `report_notes`.

**D3. Transitions.** A `Transition` enum per table row; each runs `UPDATE reports SET status =
:to, status_changed_at = :now, source_key = NULL … WHERE project_id = :p AND number = :n AND
status IN (:allowed_from) RETURNING id`, then the audit entry, one transaction. Zero rows →
409 (or 404 when the report does not exist in the project).

**D4. Bulk actions.** A confirmation page (GET) states the count; the POST runs one
transaction over the matching pending rows with a per-row audit entry. "From this source"
matches the chosen report's `source_key` within the project. Capped at 1000 rows per request
(the page says when more remain).

**D5. Admin routes.** `/admin/p/{slug}` (list, `?status=`), `/admin/p/{slug}/r/{n}` (detail),
POSTs under `/admin/p/{slug}/r/{n}/…` (`approve`, `reject`, `status`, `edit`, `kind`,
`notes`, `notes/{id}/edit|delete`, `delete` admin only), `/admin/p/{slug}/bulk`. `Project(Moderate)`
or `Project(ProjectAdmin)`; notes bound through the report.

**D6. Public lists.** Keyset: open, in progress, closed by `number DESC`; fixed by
`(status_changed_at DESC, number DESC)`. Cursor = base64url of the key's fixed-width bytes
(8 or 16), anything else 400. The kind label condition ("accepts feature requests or has any
public feature request") is one `EXISTS` on the view per page.

**D7. Spam purge.** A retention step deletes `status = 'spam' AND status_changed_at <= now -
30 days` (cascades take notes, screenshots and outbox rows).

## Risks / Trade-offs

- [Hard delete cannot be undone] → Admin only, confirmation, audited; backups.
- [Bulk reject of 1000 rows holds the writer briefly] → One transaction, capped.

## Migration Plan

Migration 6 adds notes and views. Rollback before a release: revert.
