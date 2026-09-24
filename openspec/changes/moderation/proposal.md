# Proposal

## Why

Submitted reports are private until someone acts on them, and the public needs to see
approved ones, what is being worked on and what was fixed or implemented. After this change
Kohaku is a usable tracker (design §15 item 6: "after 7 the tracker is usable").

## What Changes

- The status lifecycle with the transition table, spam purge after 30 days, and `hidden`.
- Maintainer pages per project: report lists by status, report detail, single and bulk
  actions, edits, kind changes, notes; admin-only permanent deletion; counts on `/admin`.
- Public views, public lists with tabs and a kind filter and labels, the Fixed / Implemented
  section, detail pages and the read API with keyset pagination.

## Capabilities

### New Capabilities

- `moderation`: transitions, maintainer pages and actions, visibility, public pages and read
  API.

### Modified Capabilities

- `audit-log`: report and note actions.
- `data-storage`: retention deletes old spam.

## Non-goals

- Mail to maintainers about new reports (`notifications`) and to reporters about status
  changes (`reporter-verification`).
- Screenshots on pages (`screenshots`), +1 and the "most wanted" sort (`plus-one`).
- Public comments, assignees, labels, search.

## Security considerations

- **Attacker: reads private reports.** One set of views defines "public"; every public read
  goes through them and serializes dedicated public structs; private and nonexistent reports
  answer identically.
- **Attacker: a maintainer reaching into another project.** Every admin lookup binds the
  guard's project id and the per-project number.
- **Attacker: stale or forged transitions.** Conditional writes on the current status; illegal
  transitions refused.
- **Attacker: crafted list parameters.** Strict cursor, status and kind parsing (400).
- **Attacker: script in reports and notes.** The markdown capability's sanitizer; pending
  reports show links as inert text to maintainers.

No new dependencies.

## Impact

Migration 6 (`report_notes`, views `public_reports`, `public_notes`). Admin and public
templates, the read API, one module for all public reads.
