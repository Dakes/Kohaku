# Design

## Context

Builds on `report-submission` (the insert), `users-and-invites` (grants, disable) and the
outbox. Design §9 Maintainers is the source.

## Decisions

**D1. Schema (`migrations/0007_notifications.sql`).** `notify_optouts(user_id → users
CASCADE, project_id → projects CASCADE, PRIMARY KEY (user_id, project_id))`;
`notify_sent(user_id, project_id, last_sent_at, PRIMARY KEY (…))` with both cascades;
`ALTER TABLE outbox ADD COLUMN project_id INTEGER REFERENCES projects ON DELETE CASCADE`
(foundation D31.4).

**D2. Coalescing.** In the submission's transaction, for each recipient: `UPDATE outbox SET
body = :body WHERE kind = 'notify.reports_waiting' AND user_id = :u AND project_id = :p AND
unsent`; if no row changed, `INSERT` one with `next_attempt = max(now, last_sent_at + 600)`.
The worker sets `notify_sent.last_sent_at` in the transaction that records the row as sent.
Recipient selection is one query (admin ∪ granted maintainers, enabled, with a password,
not opted out).

**D3. Opt-out deviation.** §4 put the switch in `project_members.notify`; the admin has no
grant rows, so opt-outs are their own table (owner requirement: the admin gets mail).

## Risks / Trade-offs

- [The count in a delayed mail is at most as old as the last submission] → It is updated by
  every submission until sent.
- [Updating an unsent row's body races with the worker] → The worker's result is written by
  row id; an update after the attempt started only changes a row about to be recorded as sent.

## Migration Plan

Migration 7 adds two tables and an outbox column. Rollback before a release: revert.
