# Design

## Context

Builds on `foundation` (limiter, public-write permits, route table with the JSON exemption
declaration), `projects` (project router, `features_enabled`, text rules) and
`users-and-invites` (rekey). Design §6 is the source.

## Goals / Non-Goals

**Goals:** nothing expensive before the cheap checks; the report insert itself enforces both
caps.

**Non-Goals:** see proposal.

## Decisions

**D1. Schema (`migrations/0005_reports.sql`).** `reports`: `id INTEGER PRIMARY KEY
AUTOINCREMENT`, `project_id → projects CASCADE`, `number`, `kind CHECK (kind IN
('bug','feature'))`, `title`, `body`, `status CHECK (status IN ('pending','open',
'in_progress','fixed','closed','spam','hidden'))`, `close_reason`, `plus_one_count INTEGER
DEFAULT 0`, `source_key BLOB`, `approved_at`, `status_changed_at`, `created_at`,
`updated_at`; `UNIQUE (project_id, number)`; index `(project_id, status, source_key)` for
the caps. `plus_one_count` and `close_reason` are used by later changes; the table is final
here so they need no rebuild.

**D2. Insert.** One writer transaction: `UPDATE projects SET next_number = next_number + 1
WHERE id = :p RETURNING next_number - 1`, then `INSERT INTO reports … SELECT … WHERE (SELECT
count(*) … source_key = :k AND status = 'pending') < 5 AND (SELECT count(*) … status =
'pending') < 500`; zero rows → rollback (the number is not consumed) and the same answer as
the early check.

**D3. Proof of work.** Challenge = base64url of `purpose(1) ‖ project_id(8) ‖ report(8,
0 unless plus_one) ‖ nonce(16) ‖ difficulty(1) ‖ expiry(8) ‖ MAC(32)` under
`keys::Purpose::Pow` with a `PerBootKey`. Solution: decimal counter; check `SHA-256(challenge
bytes ‖ counter as ASCII)` leading zero bits. Used set: `Mutex<HashMap<[u8;16], u64>>` keyed
by nonce, pruned by the scheduler sweep and on insert when over the cap. Difficulty
constants live in `pow.rs` with the measurement (device, browser, p95) in a comment;
starting value 18 bits for `submit`, adjusted to the 4 s target before release.

**D4. Scripts.** `static/submit.js` reads the challenge from a `data-` attribute of the form,
starts `static/pow-worker.js` (a small synchronous SHA-256, no WebCrypto per attempt), fills
the hidden `pow` field and enables the submit button when solved; errors re-render the form
with a new challenge, which the script solves again. Loaded as hashed assets; `worker-src`
falls back to `script-src 'self'`, so the CSP is unchanged.

**D5. Markdown.** `pulldown-cmark` with `ENABLE_TABLES | ENABLE_STRIKETHROUGH`; an event
filter maps `Html`/`InlineHtml` to `Text`, drops `Image` start/end keeping its text, rewrites
links failing the scheme rule to text, and counts nesting depth; `push_html` into a buffer
aborted past 262 144 bytes; then one `ammonia::Builder` (tags: p, br, hr, h1–h6, blockquote,
ul, ol, li, pre, code, em, strong, del, a, table, thead, tbody, tr, th, td; `a[href]` only;
schemes http, https, mailto; `url_relative(Deny)`; fixed `link_rel`). Rendering runs in
`spawn_blocking`. The inert-link mode for pending reports is a flag of the same function.

**D6. No page-side JSON (deviation from §6 "the page JS calls the same JSON endpoints").**
The release CSP has no `connect-src`, so page script cannot call the API; loosening it for
our own endpoints would also let injected script exfiltrate. The form submits normally with
the solved challenge; `plus-one` and `reporter-verification` follow the same pattern. The JSON
API is for in-app clients only.

**D7. Kind choice.** Two radio buttons without a default when the project accepts feature
requests (an explicit choice gives maintainers better data); none otherwise, and the report
is a bug.

**D8. Errors.** JSON error codes: `invalid_json`, `unsupported_media_type`, `invalid_field`
(with the field name), `pow_invalid`, `rate_limited`, `too_many_pending`,
`project_not_accepting`, `busy`. HTML re-renders keep title and body, never the honeypot.

## Risks / Trade-offs

- [PoW costs legitimate reporters seconds] → Solved while they type; p95 target 4 s.
- [Restart invalidates outstanding challenges] → The form gets a new one on its next submit.
- [`ammonia` brings `html5ever`] → Well maintained; the alternative is a hand-written
  sanitizer, which the design rejects.
- [A per-network cap hits users behind one NAT] → 5 pending per project; moderation frees
  slots.

## Migration Plan

Migration 5 adds `reports`. Rollback before a release: revert.
