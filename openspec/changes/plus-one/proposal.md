# Proposal

## Why

A counter shows which bugs hurt the most people and which features are most wanted, without
comments or accounts (design §6 +1, §15 item 10; owner: also sort by it).

## What Changes

- "+1" on open and in-progress public reports of projects with `plus_one_enabled`: form and
  API, a light proof of work bound to the report, the `plus_one` rate class.
- One count per network (/32 or /56) per report per day, via an HMAC under a memory-only key
  rotated every 24 hours.
- Counts on lists and detail; `sort=wanted` on the Open and In progress tabs and API.

## Capabilities

### New Capabilities

- `plus-one`: counting, showing counts, most-wanted sort.

### Modified Capabilities

- `moderation`: the `sort` parameter in public lists.
- `request-limits`: the `plus_one` class.

## Non-goals

- Down-votes, per-user vote lists, votes on fixed or closed reports, showing who voted.

## Security considerations

- **Attacker: inflating counts.** Proof of work per vote, `plus_one` bucket (30 per hour per
  network), one count per /56 per report per day; counts are a signal, not a guarantee.
- **Attacker with a backup: de-anonymizing voters.** Voter hashes use a key that never
  leaves memory and changes daily; old rows are deleted at rotation.

No new dependencies.

## Impact

Migration 10 (`plus_ones`). Button, count display, sort links, the epoch key.
