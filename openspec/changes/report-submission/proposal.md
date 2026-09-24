# Proposal

## Why

The point of Kohaku: anyone can report a bug, or request a feature where the project allows
it, without an account. This is the largest unauthenticated write surface, so every cost
and limit comes first, then the one pending row (design §6 Submission, §15 item 5).

## What Changes

- `reports` table (kind, number, title, Markdown body, status, source key, times).
- Report form at `{base}/new`, a "received" page, and `POST /api/v1/reports` with
  `GET /api/v1/challenge` for in-app clients.
- Proof of work (per-boot MAC, purpose- and project-bound, single use, 200 000-entry used
  set) solved in a web worker; honeypot; `submit` rate class; 5 pending per network and 500
  per project.
- Bug or feature request choice when the project accepts feature requests.
- Strict Markdown with post-render sanitizing.
- `admin rekey` also clears source keys.

## Capabilities

### New Capabilities

- `report-submission`: reports, submission routes, check order, proof of work, field rules,
  pending caps.
- `markdown`: accepted syntax and sanitized rendering (also used for notes).

### Modified Capabilities

- `request-limits`: the `submit` class.
- `users-and-invites`: `admin rekey` clears pending reports' source keys.

## Non-goals

- Showing reports (lists, detail pages, read API) and moderating them (`moderation`).
- Screenshots (`screenshots`), reporter email verification (`reporter-verification`),
  maintainer mail about new reports (`notifications`), +1 (`plus-one`).
- CAPTCHAs, accounts for reporters, editing a report after submission.
- Calling the JSON API from the page itself (see design D6).

## Security considerations

- **Attacker: spam bot.** Proof of work per submission (a cost), honeypot, `submit` bucket
  (10 per hour per network), 5 pending per network per project and 500 per project (the
  limits); nothing is public before a maintainer approves it.
- **Attacker: script injection through Markdown.** Raw HTML becomes text, images are dropped,
  only http/https/mailto links survive, then an allow-list sanitizer; rendered per view.
- **Attacker: resource exhaustion.** Checks before the body is read, 64 KiB bodies, nesting
  and render-size caps, a bounded used set (503 when full), 8 public-write permits.
- **Attacker: replaying or forging proofs of work.** MAC under a per-boot key, bound to
  purpose and project, 10-minute expiry, single use.
- **Attacker with a backup.** The source key is an HMAC under a secret-derived key, kept only
  while pending; no addresses are stored.

New dependencies:

- `pulldown-cmark` (default features off): CommonMark parser with an event stream we filter.
- `ammonia`: allow-list HTML sanitizer after rendering (brings `html5ever`, reviewed in
  `docs/dependencies.md`).
- axum `json` feature (first JSON body), which brings `serde_json` use for requests.

## Impact

Migration 5 (`reports`). Public templates `new`, `received`; static `submit.js` and
`pow-worker.js`; the challenge key and used set; the Markdown module.
