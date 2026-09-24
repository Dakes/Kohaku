# Proposal

## Why

The audit log has recorded every state change since `foundation`; the admin needs to read it
(design §8 Pages, §15 item 11).

## What Changes

- `/admin/audit`: admin-only, newest first, paginated, filterable by action and target, with
  current names looked up at render time.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `audit-log`: viewing it.

## Non-goals

- Export, search in free text, retention changes, maintainer access.

## Security considerations

- **Attacker: a maintainer reading others' activity.** Admin only, 404 otherwise.
- **Attacker: injection through filters.** Strict parsing (400); values are only ever bound
  as parameters and escaped when rendered.

No new dependencies.

## Impact

One admin route and template; no migration.
