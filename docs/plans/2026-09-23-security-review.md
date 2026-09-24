# Kohaku — Design security review

Date: 2026-09-23 · Subject: [2026-09-23-kohaku-design.md](2026-09-23-kohaku-design.md)

## Method

- **Round 1:** six independent threat-model reviewers (public abuse, auth, authz/privacy, injection/parsing, supply chain/release, crypto/ops), each checked by an adversarial skeptic told to refute, plus a completeness critic with its own skeptic. 77 confirmed, 8 refuted.
- **Round 2:** targeted review of custom domains and the Compose deployment, same skeptic step. 12 new findings confirmed.
- **Consolidation:** 89 findings merged into 39 design changes. Every confirmed finding is covered by exactly one change or was already resolved by the design at the time.
- **Owner decisions** where a finding was a product or infra trade-off, listed per change below.

## Changes

### `image-memory` — Bound screenshot memory from header check through encode (high)

**Owner decision:** Keep 2 concurrent jobs (owner); all other fixes applied.

Replace the Screenshots bullet with this pipeline.
(1) Checks before image work: rate limit, backstop, per-source cap, honeypot, PoW and email token all pass before the first file byte is read (see body-caps-and-order).
(2) Header check per file: check magic bytes (PNG/JPEG/WebP). Read only the header with `ImageReader::with_guessed_format()?.into_decoder()` → `dimensions()` and `color_type()`. Reject with 422 unless all of these hold: each side ≤ 16384; w×h ≤ 24 MP (down from 40 MP); w×h×bytes_per_pixel(color_type) ≤ 128 MiB. A 16-bit image near the cap therefore fails.
(3) Set `image::Limits` explicitly on every decoder: max_image_width/height 16384, max_alloc 128 MiB. Decode the first frame only.
(4) Downscale with `DynamicImage::thumbnail` to ≤ 2560 px on the longest edge. It has no Rgba32F intermediate, unlike `resize`. Then call `to_rgba8` on the small result.
(5) One global tokio Semaphore with 1 permit (`KOHAKU_IMAGE_WORKERS`, default 1), held from decode through encode. Acquire with `try_acquire`; when busy, answer 503 with Retry-After. The work runs in `spawn_blocking`. The release profile keeps `panic = "unwind"`, so a decoder panic fails one request.
(6) Document peak RSS as permits × 128 MiB plus upload buffers. Set the kohaku service `mem_limit: 512m` in compose.yaml.
(7) If libwebp stays (see libwebp): list `webp::Decoder` and `webp::Encoder::encode` in clippy.toml disallowed-methods/types and use `encode_simple`, which returns a Result.
(8) Tests:
- An 8000×5000 RGBA16 PNG (about 1.5 MB) is rejected before decode.
- A 24 MP 8-bit JPEG succeeds with peak RSS < 200 MB.

This is lighter than the reviewers' 9-point list: no per-stage permits and no custom allocator accounting.

**Applied as** (design §3 bounds, §6 Screenshots): 2 fixed permits (no `KOHAKU_IMAGE_WORKERS`); header budget and `max_alloc` 96 MiB, not 128 MiB, so two jobs fit; for progressive JPEGs the header budget also charges zune-jpeg's coefficient planes (found by verification: `max_alloc` does not see them), via our own SOF marker walk; `thumbnail` only above 2560 px (it upscales), then `into_rgba8`; per-job peak ≤ 160 MiB, documented peak ≈ 522 MiB with argon2, `mem_limit: 640m`; the full clippy ban list is in §6 step 7; memory tests measure `VmHWM` in a child process.

New dependency: none

Covers:

- Screenshot pipeline can use about 0.6 GB per image, and the 2-permit limit covers only encoding
- A small PNG can decode to hundreds of MB, and the semaphore limits only encodes

### `release-environment` — Gate publishing behind a GitHub environment, a scoped sandbox token and immutable tags (high)

**Owner decision:** Option A: full set (environment + required reviewer, tag ruleset, scoped sandbox token, immutable tags).

(1) Create a GitHub environment `release`:
- DOCKERHUB_USERNAME and DOCKERHUB_TOKEN exist only as environment secrets. Delete any repository-level copies.
- Deployment refs are limited to tags matching `[0-9]*.[0-9]*.[0-9]*`.
- The owner is the required reviewer.
- Only the publish job references it.
(2) Add a tag ruleset for that pattern that blocks update and deletion, with no bypass actor.
(3) Give the dev sandbox and agents a fine-grained PAT for Dakes/Kohaku only, set with `sbx secret set github`:
- Contents RW, Pull requests RW, Metadata R.
- No Workflows, Actions, Environments/Deployments, Administration or Secrets permission.
- Code in the sandbox then cannot push workflow files, approve deployments or change settings.
- Release tags are pushed from the host.
(4) Reword the Docker Hub part of §10: 'Read & Write personal access token (never Delete), with an expiry (≤ 1 year), stored only as a `release` environment secret. PATs are account-wide on a personal namespace.' If the dakes namespace holds other repos that matter, publish from a dedicated namespace. Turn on Docker Hub 'Specific tags are immutable' with `^[0-9]+\.[0-9]+\.[0-9]+$`. X.Y, X and latest stay mutable by design.
(5) Update the Releases section of AGENTS.md: a tag push starts the release, and the owner approves the `release` deployment.

New dependency: none

Covers:

- Any credential that can push a tag, including the dev sandbox's injected GitHub token, can publish an official signed release; the tag pattern is also inconsistent
- The Docker Hub token cannot be scoped to one repository as the design assumes; published version tags stay overwritable
- The Docker Hub token probably can't be limited to one repository on a personal account, and anyone who can push a tag can reach it

### `migration-runner` — Safe migration runner: FK-off rebuilds, foreign_key_check, pre-migration copy, downgrade guard (medium)

Make these rules part of the migration runner, not of individual migrations.
(1) Before applying anything, write `VACUUM INTO /data/backups/pre-migrate-v{old}-{unixtime}.db` with mode 0600. Keep the 2 newest such files and delete older ones.
(2) Downgrade guard: exit with a clear error when `PRAGMA user_version` is higher than the newest migration the binary knows.
(3) Per migration:
- `PRAGMA foreign_keys=OFF` outside any transaction. Bundled libsqlite3-sys defaults to foreign keys ON, and the pragma has no effect after BEGIN.
- BEGIN; apply the migration and the user_version bump in one transaction.
- Run `PRAGMA foreign_key_check` and roll back and exit if it returns any row.
- COMMIT, then `PRAGMA foreign_keys=ON`.
(4) Test: for each version k, apply migrations 1..k, insert one row per table, apply the rest, then assert that per-table row counts are unchanged and foreign_key_check is empty. No stored fixture files.
(5) README: to roll back, stop the service, restore the pre-migrate file with `kohaku restore` (see backup-restore), and pin the previous exact tag.

New dependency: none

Covers:

- Automatic forward migrations with no downgrade guard: rolling back the image can publish reports that a newer version hid
- An auto-applied table-rebuild migration silently cascade-deletes child rows, because bundled SQLite turns foreign keys on for every connection

### `body-caps-and-order` — Per-route body caps and a check order that reads no body before cheap checks (medium)

Replace the §5 check order with:
1. IP rate limit (per /64 and /48). Needs no body.
2. Project lookup, backstop and per-source pending cap.
3. Read the body under the route's cap.
4. honeypot → PoW → email token → field validation → screenshots → insert pending → queue notification.

Body caps, with axum `DefaultBodyLimit` per route:
- 64 KiB default on every route: forms, JSON API, login, admin.
- Only the HTML submit route of a project with screenshots_enabled accepts multipart, with a 25 MiB total cap.
- Multipart gets 415 everywhere else. The JSON API never accepts screenshots.

On the multipart route:
- The form puts the text fields (honeypot, PoW, email token, title, body) before the file inputs in DOM order. They are verified before the first file byte is read. A file part that arrives first gets 400.
- Read every field with `field.chunk()` and a running counter: text fields ≤ 64 KiB, files ≤ 8 MiB, ≤ 3 file parts, ≤ 12 parts in total.
- A tokio Semaphore of 4 permits bounds concurrent multipart submissions (`try_acquire`, else 503), so upload buffering stays ≤ about 100 MiB.

Honeypot hits get the normal success response.

New dependency: none

Covers:

- 'Cheapest first' is not cheap for multipart: the rate limit runs after the body is read, and one body cap serves every route
- Body limit and check order let unauthenticated clients make the server buffer about 25 MB before any cheap check

### `client-ip-and-limiter-keys` — One specified client-IP algorithm and bounded /64+/48 limiter keys (medium)

Specify one client-IP module:
(1) Canonicalize every address with `IpAddr::to_canonical()` before CIDR matching, masking or hashing. IPv4-mapped peers from a [::] listener are the reason.
(2) `KOHAKU_TRUSTED_PROXIES` is required and has no preset. It lists exact proxy addresses (/32, /128); CIDRs are allowed but documented as a last resort. The literal `none` means direct exposure.
(3) Read only X-Forwarded-For, never Forwarded, X-Real-IP, X-Forwarded-Host or X-Forwarded-Proto.
- Join all XFF headers into one list and walk it right to left, skipping trusted entries. The first untrusted entry is the client.
- If any entry is malformed, ignore the header and use the peer.
- XFF from an untrusted peer is ignored.
(4) Limiter keys: IPv4 /32. IPv6 uses a /64 bucket plus a /48 aggregate bucket in the same map, with 8× the /64 budget.
(5) Every 60 s, sweep map entries that have refilled to full. Cap the map at 100 000 entries; past the cap, new keys share one overflow bucket per action class.
(6) Log two one-time warnings, without IPs: 'XFF from untrusted peer' and 'trusted peer sent no XFF'.
(7) Unit tests:
- nginx-style forged prepended entry
- mapped IPv4 peer
- XFF from an untrusted peer
- two XFF headers
- a malformed entry

README: an nginx snippet with `proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;` (or NixOS `recommendedProxySettings`), and a note that Kohaku's port must never be published beyond the proxy network. About 40 lines of code. Skip the reviewers' optional admin diagnostics panel.

New dependency: none

Covers:

- Client-IP derivation is underspecified, and four common traps collapse or bypass every IP-keyed control
- Client-IP extraction leaves open which X-Forwarded-For entry to use, what the default trust list is, and how IPv4-mapped addresses are handled; all three can fail open
- Per-/64 buckets alone are beaten by /48 rotation, and the bucket map has no bound

### `compose-proxy-network` — Dual-stack proxy network with fixed Caddy addresses kept next to KOHAKU_TRUSTED_PROXIES (medium)

compose.yaml:
(1) Set `enable_ipv6: true` on the `proxy` network, with a fixed ULA /64 next to the IPv4 subnet. IPv6 clients then reach Caddy through DNAT with their real address, instead of being relayed by docker-proxy as the gateway.
(2) Give Caddy a fixed `ipv4_address` and `ipv6_address`. KOHAKU_TRUSTED_PROXIES lists exactly those two addresses.
(3) The subnets, Caddy's addresses and KOHAKU_TRUSTED_PROXIES are adjacent literals in compose.yaml, with KOHAKU_TRUSTED_PROXIES in the kohaku `environment:` block, not in .env. One README sentence names these values as a set that changes together.
(4) Default IPv4 subnet: a /29 outside Docker's default pools (172.17–31.0.0/16, 192.168.0.0/16) and away from common LAN ranges, e.g. 10.231.7.0/29. The ULA /64 is generated once and hard-coded.
(5) README requires Docker Engine ≥ 27, which enables ip6tables by default.
(6) Mandatory one-time warning, without IPs, when the resolved client address is loopback, RFC 1918, ULA or link-local. In the shipped setup no real client has one, so it flags gateway collapse or config drift.
(7) Check once by hand on a dual-stack host and record the result in the PR. No CI assertion.

New dependency: none

Covers:

- Every IPv6 visitor reaches Kohaku as the Docker gateway address, so they all share one rate-limit key and one me-too vote
- The fixed subnet can silently shadow a host route or fail with a pool overlap, and it is written a second time in KOHAKU_TRUSTED_PROXIES

### `origin-check` — Referrer-Policy same-origin plus one Fetch-Metadata/Origin middleware on every POST, public and admin (medium)

(1) Set `Referrer-Policy: same-origin` on every response. no-referrer makes browsers send `Origin: null` on same-origin form POSTs. External links keep rel=noreferrer, so nothing leaks cross-origin.
(2) One middleware on every non-GET/HEAD request, admin and public:
- If `Sec-Fetch-Site` is present and not `same-origin`: 403.
- Otherwise, if Origin is present it must equal the expected origin exactly. `null` or any other value: 403.
- If both headers are absent: admin, login, reset, setup and HTML-form routes get 403; JSON API writes are allowed, because native clients send neither.
(3) The expected origin comes from the matched host entry: KOHAKU_BASE_URL's origin on the main host, `https://{public_host}` on a project host. Never from Host or forwarded headers.
(4) JSON API writes require `Content-Type: application/json`, so a cross-site browser write needs a preflight. Preflights for non-GET never get CORS headers, so CORS `*` stays read-only.
(5) Admin forms keep the per-session CSRF token: a random value on the session row, no HMAC key needed.
(6) Route-matrix cells for every POST route: `Origin: null` → 403, `Sec-Fetch-Site: cross-site` → 403. A project-host form POST with `Origin: https://{public_host}` and no Sec-Fetch-Site → accepted.
(7) Foundation acceptance criteria: one manual real-browser login POST and one real-browser report POST.

About 30 lines.

New dependency: none

Covers:

- The global `Referrer-Policy: no-referrer` makes browsers send `Origin: null` on every form POST, which breaks or weakens the Origin check
- Public POST endpoints accept cross-site requests, so any web page can turn its visitors into PoW solvers with fresh residential IPs
- `Referrer-Policy: no-referrer` makes browsers send `Origin: null` on every form POST, which breaks the planned Origin check

### `object-scoping` — Two named guards, nested admin routes and project-bound child lookups (medium)

(1) Name two guards in §4:
- `RequireAdmin` for global admin routes.
- `ProjectAccess<Cap>` with a capability (Moderate | ProjectAdmin) declared in the route table.
(2) Nest every admin route that acts on a report, note or screenshot under `/admin/p/{slug}/r/{number}/…`.
- Handlers get project_id only from ProjectAccess.
- Child queries bind it: `WHERE project_id=? AND number=?`, with notes and screenshots joined through reports.
- Nothing is looked up by global id alone, and no hidden report_id form fields.
(3) The dashboard and any cross-project list filter by the user's project_members rows. The admin sees everything.
(4) New matrix cells:
- A maintainer who is a member → 404 on admin-only routes.
- A member of A addressing B's child through A's slug → 404.
- The dashboard shows no rows from ungranted projects.

Skip the full cartesian matrix.

New dependency: none

Covers:

- ProjectAccess and the route matrix don't cover maintainer→admin or cross-project object access
- ProjectAccess checks the project in the URL, not whether the report, note or screenshot belongs to it (cross-project IDOR)

### `token-flows` — One purpose-scoped token table, GET-safe landing pages, reset that never bypasses 2FA, revocable invites (medium)

(1) Replace `invites` with one `tokens` table: purpose (invite|reset|setup), SHA-256(token), user_id or email, grants for invites, expires_at, used_at. Every lookup includes purpose.
(2) A GET on a token URL only validates and renders a form. The POST consumes the token atomically with `UPDATE tokens SET used_at=? WHERE hash=? AND purpose=? AND used_at IS NULL AND expires_at>?` and checks rows affected.
(3) Invites:
- Acceptance only INSERTs a new user, relying on the unique email constraint.
- It re-reads the invite in the same transaction and drops grants to projects that no longer exist.
- The admin users/invites page lists pending invites with a Revoke action (deletes the row, audited).
(4) Password reset:
- Completing it never creates a session and never clears or skips TOTP.
- It revokes all of the user's sessions and other reset tokens.
- Requests for unknown or disabled users return the same response and do the same work: always enqueue an outbox job, and the worker drops unknown addresses.
- Limit: 3 per hour per normalized address, plus the global budget in public-mail-abuse.
(5) Web reset is disabled for the admin role. `kohaku admin reset-password --email X` prints a 1 h reset link built from KOHAKU_BASE_URL.
(6) TraceLayer spans record only axum `MatchedPath` and the status, never the URI or query. README: proxy access logs contain token URLs and IPs, so keep their retention short.

Skip the reviewers' address re-entry step and cookie redirect.

New dependency: none

Covers:

- Password reset can skip 2FA, gives an email-only path to the admin account, and its timing reveals which emails have accounts
- Invite/reset links: mail scanners use them up on GET, tokens end up in proxy logs, and the flows are loosely separated
- Secret tokens in URL paths will reach logs despite the 'no tokens in logs' rule
- Pending invites cannot be revoked

### `login-flow` — Single-step login, bounded argon2 queue, re-auth for sensitive changes (medium)

(1) One login form with email, password and an optional TOTP or recovery code.
- A wrong password, a wrong code, or a missing code for an enrolled account all return the same generic error and count as one failure toward the per-IP and per-account limits.
- There is no half-authenticated state. A new session token is issued only after all factors pass.
(2) Check order: 64 KiB body cap → per-/64 and per-/48 login bucket → account lock check → argon2 semaphore `try_acquire` with ≤ 2 s wait (else 503) → re-check the lock after acquiring → argon2 → TOTP.
(3) Changing the password, disabling or re-enrolling 2FA, and regenerating recovery codes require the current password, plus a TOTP code if enrolled.
(4) Enabling 2FA revokes the user's other sessions. Turning on 'require 2FA for maintainers' revokes all sessions of non-enrolled maintainers, whose next login goes to enrollment.

No per-session mfa column and no separate pre-2FA record.

New dependency: none

Covers:

- Second-factor step has no attempt limit, no defined pre-2FA state and no session rotation, and it reveals when the password is right
- Sensitive account changes need no re-authentication, and sessions don't track 2FA state
- Login flood: random emails always pay for a full argon2 hash, and rotating IPv6 /64s get past the per-IP limit, so the admin can't get a login slot

### `lockout-policy` — Replace the hard account lock with a policy that cannot lock the owner out (medium)

**Owner decision:** Option A: device cookie.

Recommended (option A):
(1) After a full login, set `__Host-kohaku_device` (Secure, HttpOnly, SameSite=Strict, Path=/, 1 year, main host only) holding a random 256-bit token. Store SHA-256(token) in `known_devices(user_id, token_hash, created_at)`, at most 10 per user, oldest evicted. No signing key is needed.
(2) The per-account failure counter and locked_until live in `users`, so they survive restarts.
- Key them on user_id after normalizing the email (trim, ASCII-lowercase). Unknown emails only hit IP limits.
- 10 failures → 15 min lock, applied only to attempts without a valid device cookie for that account.
- Known devices get a separate budget of 20 failures per hour.
(3) While the account is locked, still run the dummy hash and return the generic error. Never show a 'locked' message.
(4) Send lockout mail at most once per account per 24 h.
(5) Add `kohaku admin unlock --email X`.

The unlock CLI, mail cap and generic error apply under every option.

New dependency: none

Covers:

- Unauthenticated lockout lets anyone keep the admin locked out and flood their inbox
- Account lockout lets anyone keep the single admin locked out, floods their mail and reveals which emails have accounts

### `key-inventory-totp` — Key table: ephemeral keys by default, one external secret that derives TOTP seeds (medium)

(1) Add a key table to §4.
- Ephemeral keys (random at boot, memory only): PoW challenges, the verified-email token, email_codes HMACs, and the me-too epoch key (rotated every 24 h).
- No key needed: CSRF (random value on the session row), unsubscribe (random per-contact token stored hashed), device cookies (random, stored hashed).
- Every MAC input starts with a purpose label and uses length-prefixed fields. Every check uses `Mac::verify_slice`.
(2) One persistent secret: required `KOHAKU_SECRET_FILE` with ≥ 32 random bytes.
- It ships as the Compose secret `secrets/kohaku_secret`. The README gives `head -c 32 /dev/urandom | base64 > secrets/kohaku_secret`.
- **Superseded (owner, 2026-09-24):** the secret is the environment variable `KOHAKU_SECRET` in `.env` (mode 0600), like the SMTP password; there are no secret files.
- It is never stored in the DB or /data.
- The DB stores HMAC(secret, 'kohaku/keycheck'), and startup fails on a mismatch.
(3) TOTP seeds are never stored. Each user has a random 16-byte `totp_nonce`, and seed = HMAC-SHA256(secret, 'kohaku/totp' ‖ user_id ‖ nonce)[..20]. Re-enrollment draws a new nonce. A leaked DB or backup alone cannot produce codes. A lost secret means `admin reset-2fa` for enrolled users (documented).
(4) Recovery codes: 10 codes of 16 base32 characters (80 bits), stored as SHA-256. They are consumed with `DELETE … WHERE user_id=? AND hash=?`, checking rows affected.
(5) Replay: `totp_last_step` on users, and a code is accepted only through `UPDATE users SET totp_last_step=? WHERE id=? AND totp_last_step<?` on the writer. TOTP failures count toward the login limits.

This is lighter than both reviewer options: no cipher crate and no hand-rolled XOR stream.

New dependency: none

Covers:

- TOTP seeds are stored in plaintext in the DB; recovery-code entropy and TOTP replay handling are unspecified
- The TOTP secret storage question is still open; stored in plaintext, a leaked backup turns a cracked password straight into admin login
- No key inventory: where HMAC keys come from, where they are stored, how uses are kept separate, and how keys rotate are all undefined

### `db-write-rules` — Atomic check-and-consume on the writer, explicit pragmas, bounded public writes (medium)

(1) Design rule: every check-and-consume and check-and-count is one conditional statement or transaction on the writer connection, decided by rows affected or RETURNING. This covers OTP attempts, login failures, the TOTP step, recovery codes, tokens and pending caps. Security decisions never come from a reader snapshot.
(2) Pragmas on every connection: journal_mode=WAL, synchronous=NORMAL, foreign_keys=ON, busy_timeout=5000, journal_size_limit=67108864. The writer also sets secure_delete=ON.
(3) Write `sessions.last_seen` at most once per 5 min per session.
(4) Public write jobs (submit, OTP, me-too) take a permit from a Semaphore of 8 (`try_acquire`, else 503). Admin writes bypass it.
(5) The background worker runs `PRAGMA wal_checkpoint(TRUNCATE)` hourly, and after the retention job and any erasure.
(6) Concurrency tests with 20 parallel requests, where at most the limit may succeed: OTP verify, login lockout, recovery code, invite/reset token, PoW nonce.

No dedicated writer thread and no priority channels.

New dependency: none

Covers:

- The reader/writer split makes every single-use and attempt-limited check racy: parallel requests multiply OTP, TOTP and login guesses
- The single-writer path has no bound or priority, so public writes can starve admin and reads

### `public-mail-abuse` — Global budget for unauthenticated mail, requester-bound OTP, scoped single-use verified-email token (medium)

(1) All unauthenticated mail triggers (OTP, password reset) share one budget: `KOHAKU_PUBLIC_MAIL_PER_HOUR`, default 60, plus 10 per hour per IPv6 /48 or IPv4 /32. When it is exhausted, answer 429 'try again later'.
(2) Per-address limits (OTP 3/h, reset 3/h) key on the lowercased address with any +tag stripped. No provider-specific dot rules.
(3) Sending a code returns a random `verification_id` to the requester. Verification needs verification_id + code, and the 5 attempts are counted per id, so a third party cannot burn someone else's code.
(4) Verified-email token:
- HMAC(ephemeral key, 'report-email' ‖ project_id ‖ verification_id ‖ email ‖ expiry), valid 30 min.
- Single use: its nonce goes into the in-memory used-set shared with PoW. It is marked just before the report insert and released if the insert fails.
- The handler takes reporter_email only from the token, never from a form field.
(5) Outbox:
- Rows get `expires_at`. OTP mail expires with its code (10 min) and is never retried after that.
- Security mail is sent before other mail.
- Lockout mail goes out at most once per account per 24 h.

**Applied as:** the outbox worker and the public mail budget are built in `foundation` (design §15), since every later change sends mail through them. The single-use key of the verified-email token is its `verification_id`, kept in the used-set for the token's 30 min; verification also takes the email.

New dependency: none

Covers:

- Public mail triggers lack a global budget, the per-address limit is bypassed by aliases, and a third party can burn a victim's OTP
- The 'verified email' token has no defined scope: one verification gets past require_email everywhere for 30 minutes, and a substituted address turns Kohaku into a mail relay

### `backstop` — Per-source pending cap and bulk moderation so the backstop is not an off switch (medium)

(1) Moderation gets three bulk actions, with confirmation: 'reject selected as spam', 'reject all pending from this source' and 'reject all pending'. Rejecting as spam deletes screenshots immediately.
(2) Pending rows store `source_key` = HMAC(KOHAKU_SECRET-derived 'kohaku/source' subkey, IPv6 /48 or IPv4 /32). Because the secret lives outside the DB, a DB leak cannot reverse it. Clear source_key on approve or reject.
(3) Cap pending reports per source per project at 5 (configurable). The form answers 'too many pending reports from your network' when a source hits it.
(4) The project-wide backstop (500) still closes the form. Optional, one branch: when SMTP is configured, it switches the project to require_email instead of closing.

Skip reserved shares and multi-stage degraded modes.

New dependency: none

Covers:

- The pending backstop is an off switch: one attacker can close a project's report form indefinitely

### `screenshot-storage` — Screenshots as SQLite BLOBs, immediate deletion on spam, media quotas, rotated logs (medium)

(1) Store the re-encoded WebP as a BLOB, the last column of `screenshots`, instead of files under /data/media.
- Cascades, purge, erasure and `kohaku backup` then cover images automatically.
- No path handling, temp-rename or orphan sweeper is needed.
- Remove /data/media from §3 and the README.
(2) Rejecting a report as spam deletes its screenshots immediately. Only the text is kept for the 30-day spam window.
(3) Quotas computed from SUM(length(bytes)): 256 MiB per project and 1 GiB for the instance, both configurable. Above quota the form accepts text only, and the admin dashboard shows it.
(4) Log rejected or abusive requests as per-minute counters per reason, not one line per request.
(5) compose.yaml: `logging: driver: local` on both services, or json-file with max-size 10m and max-file 3.

New dependency: none

Covers:

- No storage quota: spam screenshots kept 30 days, orphaned media files, unbounded container logs
- Screenshot files on disk are not removed by purge or delete, so spam images stay forever

### `markdown-pipeline` — Depth and output caps in the event filter, and an exact ammonia builder (medium)

(1) In the existing pulldown-cmark event filter:
- Reject the submission with 422 when BlockQuote/List/Item/Table nesting exceeds 16.
- Abort and reject when the rendered HTML exceeds 256 KiB, before it reaches ammonia.
- Links pass only if they parse as absolute URLs with scheme http, https or mailto. That excludes `//host`, relative links and other schemes.
- Images are removed and raw HTML becomes text.
(2) Specify the exact `ammonia::Builder`:
- Tags limited to what the enabled pulldown-cmark options emit.
- `a` gets only `href`.
- `url_schemes` {http, https, mailto}, `url_relative(Deny)`.
- `link_rel(Some("nofollow noopener noreferrer ugc"))`: ammonia sets rel, not the filter, because ammonia rewrites it otherwise.
(3) Rendering runs in `spawn_blocking`, on each view. There is no cached-HTML column: depth-capped rendering costs < 1 ms, and sanitizer fixes then apply to old reports.
(4) Add img, `tel:`, `//evil.example`, h1 and deep-nesting probes to the XSS corpus. Run the fuzz target with `-timeout=2 -max_len=20480`.

Showing links as inert text in the pending view is optional.

New dependency: none

Covers:

- Deeply nested markdown costs about 1 s of CPU in ammonia per 20 KB submission
- ammonia's defaults contradict the design, and pending reports render live links inside the admin UI

### `public-read-bounds` — Paginate every public list and add a read limiter class (medium)

(1) Every public list has a hard page size of 50 with keyset pagination: open, in progress, closed and the permanent Fixed section, in both HTML and `/api/v1`. The cursor is (sort_key, number), and the API returns `next`.
(2) List pages render titles only. Markdown bodies render only on the detail page, per view (see markdown-pipeline). This rejects the render-on-write + renderer_version column as heavier than needed.
(3) Add a 'read' class to the existing limiter: 300 requests/min per key, with /static, /healthz and tls-ask excluded.

New dependency: none

Covers:

- The public read path is unbounded: no page size, no read rate limit, and possibly markdown rendered on every request

### `unpublish-approved` — Let approved reports be hidden, edited or deleted (medium)

**Owner decision:** Option A: hidden status, audited edits, admin hard-delete.

Recommended (option A):
(a) Add a private `hidden` status:
- Reachable from every state, and able to return to open.
- Never purged, not counted toward the backstop, excluded by the allowlist views (see visibility-allowlist).
(b) Maintainers can edit the title, body and notes of reports on granted projects. Every edit is audited, with ids only.
(c) The admin can hard-delete a report. It cascades to notes, screenshot BLOBs, me_too rows, report_contacts and queued outbox rows.
(d) Key the spam purge on a new `status_changed_at` column.
(e) Optional: a per-project line on the form, 'report security issues privately to …'.

Add the transition table to the moderation spec. The downgrade guard (migration-runner) and the allowlist make the new status safe to add.

New dependency: none

Covers:

- Approved reports can never be made private, redacted or deleted, and there is no private state that is kept

### `release-workflow` — Split build and publish jobs, no caches in release, sign and verify by digest, useful SBOM (medium)

(1) Use two jobs.
- `build` (matrix x86_64/aarch64 musl): `permissions: contents: read`, no secrets, no id-token, checkout with `persist-credentials: false`, no cache restore. Runs `cargo zigbuild --locked --release` and uploads the binaries plus SHA-256 as artifacts.
- `publish`: `needs: build`, `environment: release`, `permissions: {contents: read, id-token: write}`. Verifies the hashes and runs docker login. The Dockerfile only COPYs the binaries onto the pinned base: no RUN, no QEMU. Pushes multi-arch, then runs `cosign sign --yes docker.io/dakes/kohaku@${DIGEST}` on the index digest from build-push-action.
- §10 states that this protects the credentials, not the binary's contents.
(2) Release jobs restore no caches. PR CI uses `pull_request` with `contents: read` and no secrets. Never use `pull_request_target` or `workflow_run`.
(3) The README documents one verify command: `cosign verify --certificate-oidc-issuer https://token.actions.githubusercontent.com --certificate-identity-regexp '^https://github\.com/Dakes/Kohaku/\.github/workflows/release\.yml@refs/tags/[0-9]+\.[0-9]+\.[0-9]+$' dakes/kohaku:X.Y.Z`. Compose stays on `:1`.
(4) SBOM: set `ARG BUILDKIT_SBOM_SCAN_CONTEXT=true` and copy Cargo.lock into the publish build context, so the attestation lists the Rust crates. Record the bundled SQLite (and libwebp, if kept) versions in docs/dependencies.md. If the first attestation still lists no crates, drop the SBOM attestation rather than ship false assurance.

New dependency: none

Covers:

- The release build runs every crate's build code in the same job that holds the Docker Hub token and the Sigstore signing token
- Keyless signing only helps if verification pins the identity; the design specifies neither the verify command nor signing by digest
- Release builds can restore caches written on main, and pull_request_target is not ruled out
- The planned SBOM will list almost none of the Rust code and none of the bundled C (SQLite, libwebp)

### `host-routing` — Two routers chosen by the host map, two exempt internal endpoints, main-host-only cookies (low)

(1) The in-memory host map chooses one of two routers.
- Main host: /admin/*, the login, invite, reset, setup and unsubscribe pages, /p/{slug}/*, /static, media.
- Project host: /, /new, /r/{n}, /api/v1/*, the OTP and me-too endpoints, /static, media. It receives the host's project_id as state and has no slug parameter, so /p/* on a project host is 404.
- Unknown hosts get 421 and are never routed to the main host.
(2) Every public lookup on a project host binds that project_id. Screenshots join through reports and the public views.
(3) Exactly two internal endpoints are matched before the host router, for any Host:
- /healthz. `kohaku healthcheck` probes 127.0.0.1.
- /.well-known/kohaku/tls-ask. Caddy's ask arrives with Host `kohaku:8080`. It uses the same normalization function as the router, answers only from the in-memory host map (no DB access), sits outside every rate-limit class, and is not logged per request. Answers: 200 for a configured host, 404 otherwise.
- The shipped Caddyfile's public site blocks add `respond /.well-known/kohaku/* 404`.
(4) Every cookie Kohaku sets is `__Host-`-prefixed and set only by main-host handlers. Project hosts never set cookies. The PoW challenge, OTP verification_id and verified-email token travel in form fields or JSON.
(5) Reword §5: `__Host-`, the CSRF token and the exact Origin match protect the admin from same-site script; SameSite does not. README: prefer an admin host on a registrable domain that no project host or other service shares.
(6) Resource isolation for /admin GETs, about 15 lines. Allow Sec-Fetch-Site `same-origin` and `none`. Allow `same-site` and `cross-site` only with `Sec-Fetch-Mode: navigate`. Requests without the header pass.
(7) Integration tests:
- B's screenshot id on A's host → 404.
- /p/b on A's host → 404.
- Invite, reset or login path on a project host → 404.
- tls-ask with Host kohaku:8080 → 200 for a configured host, 404 for an unknown one.
- A reset request with a forged Host and X-Forwarded-Host still mails a KOHAKU_BASE_URL link.

Skip the full host × route matrix, the duplicate-cookie handling and the cookie-bomb test.

New dependency: none

Covers:

- The host router returns 421 to Caddy's tls-ask call and to the healthcheck, and the likely quick fix undoes host isolation
- No per-host route table: project hosts can serve other projects' pages and screenshots, auth pages outside /admin can set the session cookie on a project origin, and the Origin check needs a per-host expected origin
- SameSite=Strict gives no protection between a project host and a same-site admin host, and only the session cookie has a defined scope

### `headers-and-cache` — Global outermost header layer with HSTS and a Cache-Control policy; strict BASE_URL validation (low)

(1) Apply the security header set as the outermost layer, above the host routers, on every response on every host, /admin included: exact CSP, Referrer-Policy same-origin, X-Frame-Options DENY, COOP same-origin, X-Content-Type-Options nosniff, empty Permissions-Policy, and `Strict-Transport-Security: max-age=31536000` (no includeSubDomains, no preload).
(2) Cache-Control:
- `no-store` on every /admin response, on the login, invite, reset and setup/enrollment pages, and on any response rendered with a session.
- `no-cache` on public HTML, /api/v1 and screenshots.
- `public, max-age=31536000, immutable` only on hashed /static.
- Admins view pending screenshots through their own route under /admin/p/{slug}/r/{n}/…, never through the public media URL.
(3) Logout is a POST with the CSRF token. It deletes the session row and expires the cookie.
(4) At startup, reject a KOHAKU_BASE_URL that is not exactly `https://host[:port]` with no path, query or fragment. `http://localhost` is allowed only with the dev feature.
(5) Tests, in a default-features build:
- The exact CSP string on a public page, and on the admin report page while it shows a pending report.
- Cache-Control per route class.
(6) Reword the §2 Domains row: the split protects admin sessions from bugs in public-only pages. For report content, the boundary is the sanitizer plus the CSP, and neither may be loosened because of the split.
(7) Deployment docs: do not enable proxy or CDN caching for /admin, /p, /api or media.

New dependency: none

Covers:

- No Cache-Control on admin, enrollment and token pages; logout not specified
- No Cache-Control policy, so admin pages and screenshots can be cached and stay reachable after unpublishing
- No HSTS anywhere, and KOHAKU_BASE_URL is not validated as https-only with no path
- The domain split does not isolate the main XSS vector, and the CSP is specified only for the public surface

### `pow-challenge` — Purpose-bound, per-boot-keyed PoW with atomic nonce consumption and a defined JS requirement (low)

(1) The signed challenge fields are:
- purpose (submit | email_code | me_too)
- project_id
- report number, for me_too only
- a 128-bit nonce
- difficulty
- expiry (10 min)

The MAC key is random per boot and never persisted, so a restart invalidates outstanding challenges instead of making spent ones replayable.
(2) The verifier rejects a mismatched purpose or project, and checks the configured difficulty for that purpose. The embedded difficulty is only a client hint.
(3) Consume nonces atomically at verification, before any expensive work, in an in-memory `Mutex<HashMap<nonce, expiry>>` pruned at expiry and capped at 200 000 entries (503 when full). Error responses carry a fresh challenge that the form solves in the background.
(4) Submission, OTP send and me-too require JS. Show a <noscript> notice. No server path accepts these actions without a valid PoW. Amend the §2 'works without JS' claim accordingly.
(5) Pick each purpose's difficulty for a p95 solve of 2–4 s on a low-end phone; measure once and document it. The worker starts solving on page load and uses a small synchronous SHA-256 in project JS, not WebCrypto per attempt.
(6) State that PoW is a cost, not a limit; quotas and rate limits are the limits. Lockout counters live in SQLite (see lockout-policy).

New dependency: none

Covers:

- PoW token is not bound to its purpose, and the consume point and replay store are unspecified
- SHA-256 PoW cannot deter GPUs without failing low-end phones, and the no-JS path is undefined
- PoW challenges are not bound to an action, and difficulty and single-use enforcement are unspecified
- A restart wipes all in-memory anti-abuse state, and an OOM kill (reachable through the confirmed decode paths) makes restarts attacker-triggerable

### `me-too-hash` — Per-report voter hash with an in-memory daily key, and visibility on the vote path (low)

(1) voter_hash = HMAC(epoch_key, 'metoo' ‖ report_id ‖ prefix).
- epoch_key is random, held only in memory, and rotated every 24 h.
- me_too rows store the epoch. Rows from other epochs are deleted at rotation and at startup; me_too_count keeps the tally.
- The prefix is IPv4 /32 or IPv6 /56, taken after `to_canonical()`.

Votes are then unlinkable across reports, not reversible from a DB copy, and limited to one per network per report per day.
(2) The me-too POST reads through the public views (see visibility-allowlist) and returns the same 404 for pending, spam, hidden and nonexistent reports.
(3) Optional: a cap of 100 increments per report per hour.

New dependency: none

Covers:

- Me-too dedup is per /64 and resets when the key rotates; the write path's visibility check is not specified
- voter_hash links voters across reports and can be reversed if the rotating key is persisted
- Me-too 'rotating key': location undefined, and a persisted key makes voters' IP addresses recoverable

### `visibility-allowlist` — Visibility as an allowlist defined once in SQLite views (low)

Replace `status NOT IN ('pending','spam')` with `status IN ('open','in_progress','fixed','closed')`, defined once as SQLite views: `public_reports`, plus `public_notes` and `public_screenshots` joined through it.

Every public and API handler reads only through these views, including the screenshot handler and the me-too write. A non-public report returns the same 404 as a nonexistent one.

Keep the public queries in one module. No SQL-scanning test and no renumbering at approval.

New dependency: none

Covers:

- The visibility predicate is a denylist, applied by convention in each query

### `cli-bootstrap` — CLI prints single-use setup links; password and TOTP are set on the web (low)

This picks the setup-link design over CLI-side enrollment, because it needs no hidden-input handling and reuses the token-flows machinery.
(1) `kohaku admin create --email X` takes no password. It refuses if an admin already exists. It creates a user who cannot log in yet and prints a single-use setup link: a purpose=setup token, stored as SHA-256, valid 1 h, built from KOHAKU_BASE_URL.
(2) The setup page sets the password and enrolls TOTP. The code must be typed back before the account works.
(3) `kohaku admin reset-2fa --email X` clears enrollment, revokes the user's sessions, writes an audit entry with actor 'cli' and prints a new setup link.
(4) Enrollment pages show the base32 secret and otpauth URI as text, with no QR. That needs no crate, and data: images are blocked by img-src 'self' anyway.
(5) Passwords are never taken from argv or env.
(6) The CI smoke test runs `admin create` non-interactively and only checks that a link is printed.

New dependency: none

Covers:

- Admin TOTP enrollment and reset-2fa leave a password-only window where whoever knows the password binds their own authenticator
- Admin bootstrap puts the password on the command line and leaves a race for first-login TOTP enrollment

### `reporter-data` — Contact data in its own short-lived table, dedicated public view structs, secure_delete (low)

(1) Move the reporter email and notify flag into `report_contacts(report_id PK REFERENCES reports ON DELETE CASCADE, email, notify, unsubscribe_token_hash)`.
- Delete the row on unsubscribe, 30 days after the report reaches fixed or closed, and on admin erasure.
- Erasure also deletes matching outbox rows.
(2) Public templates and the API serialize only dedicated `PublicReport` and `PublicNote` structs with no user or contact fields. One test asserts the API's JSON key set.
(3) Public notes carry no per-user author, only a generic 'Maintainer' label.
(4) Set `PRAGMA secure_delete=ON` on the writer. Run `wal_checkpoint(TRUNCATE)` after the retention job and after an erase. Document that earlier backups and volume snapshots still hold erased data.
(5) Fix the §4 wording: email_codes stores HMACs. Backups are created with mode 0600 (see backup-restore).

An optional privacy-notice config string can be shown on the form and the OTP step. No masking of emails from maintainers.

New dependency: none

Covers:

- reporter_email sits on the public reports row, is kept forever, and can only be erased manually and partially
- Public maintainer notes have no author field except the login email, so implementations will publish the admin's login identifier
- Purge and erase leave the deleted data in the live SQLite file

### `outbox-content` — Content-free maintainer mail and prompt deletion of token-bearing outbox rows (low)

(1) The maintainer 'new pending reports' mail contains only the project name, the count and a link to the admin queue. No titles, bodies, reporter emails or user-supplied URLs.
(2) When a grant is revoked, a user is disabled or a user opts out, delete that user's unsent outbox rows.
(3) Token-bearing mail (invite, reset, setup, OTP) is deleted from outbox in the same transaction that marks it sent or given up. When delivery gives up, the token expires too.
(4) The retention job purges every other outbox row once it is sent or past its final attempt, whatever its state.
(5) Reporter mail carries List-Unsubscribe plus `List-Unsubscribe-Post: List-Unsubscribe=One-Click`. The unsubscribe landing page renders on GET and acts only on POST.

Skip the intent-based outbox and minting tokens at send time.

New dependency: none

Covers:

- Queued maintainer notifications still reach users who lost access, and their content is unspecified
- The outbox stores raw invite and reset tokens and OTP codes, which cancels out hashing them

### `audit-early` — Audit table and helper in foundation; ids only; 1-year retention (low)

(1) Create the `audit_log` table and an `audit(actor_id, actor_label, action, target_type, target_id)` helper in `foundation`.
(2) Every later change calls it for each state-changing admin action and each CLI command (actor 'cli').
(3) Targets are ids only: never emails, titles, bodies or IPs. actor_id has no foreign key, so deleting a user keeps the history, and a label is stored next to it.
(4) The retention job purges entries older than 1 year.
(5) Change 11 becomes the audit-log viewer only.

New dependency: none

Covers:

- Audit log content and retention are unspecified, and moderation ships before the audit log exists

### `dev-feature` — Drop tower-livereload and listenfd; small same-origin reload script; enforce dev stays out of release (low)

(1) Remove tower-livereload and listenfd.
(2) Under `cfg(feature = "dev")`:
- Serve a same-origin `/static/dev-reload.js`, about 15 lines. It polls `/dev/boot-id` every second, keeps retrying while the server restarts, and reloads when the id changes.
- Serve CSS/JS from disk.
- The CSP is the release CSP plus `connect-src 'self'` (fetch needs it), defined as a cfg'd constant. There is no inline script, so no hash is needed.
(3) Add `#[cfg(all(feature = "dev", not(debug_assertions)))] compile_error!("dev feature in release build");`.
(4) The exact-CSP tests are `#[cfg(not(feature = "dev"))]`. CI runs `cargo test` with default features as the gate and keeps clippy on `--all-features`. Update the AGENTS.md checklist to run both `cargo test` and `cargo test --all-features`.
(5) The dev loop is `watchexec -r -- cargo run --features dev`. No socket passing is needed because the script retries.

New dependency: none (removes tower-livereload and listenfd)

Covers:

- The CSP blocks both the planned livereload dev loop and any fetch from the page, which invites ad-hoc loosening
- The dev feature is unenforced and not harmless: tower-livereload needs a weaker CSP and adds an endless unauthenticated stream
- Nothing enforces that the `dev` feature (tower-livereload, assets from disk) stays out of release builds

### `canonical-redirect` — 308 only for HTML GET/HEAD, cached for an hour; token links always on the main host (low)

(1) The /p/{slug} → public_host 308 applies only to GET and HEAD of HTML pages. Form POSTs and all of /p/{slug}/api/v1/* keep being served on the main host, so no request body is ever re-sent and hard-coded in-app clients keep working.
(2) The 308 carries `Cache-Control: max-age=3600`, so changing or removing a public_host takes effect within an hour instead of being cached indefinitely by browsers.
(3) Link rule in §5: token-bearing and unsubscribe links always use the main host. Report and project links use the current public_host.
(4) README: in-app clients should use the main-host `/p/{slug}/api/v1` base, which survives domain changes.

No public_host verification step and no alias table. A confirmation dialog on host change is optional.

New dependency: none

Covers:

- The main-domain 308 re-sends POST bodies (reports, reporter emails, screenshots) to a public_host nobody has verified
- Changing, removing or reassigning a project domain is undefined: browsers cache the 308 forever, mailed links die, and in-app clients follow the host into another project

### `caddy-hardening` — Pinned, hardened Caddy with persistent named volumes (low)

Settings for the caddy service in compose.yaml:
(1) Pin an explicit tag (`caddy:2.N`, or a digest), and use the same tag in the smoke test.
(2) Named volumes `caddy-data:/data` and `caddy-config:/config`. This is required: anonymous volumes lose certificates and the ACME account on `down` and can hit Let's Encrypt's 5-per-week duplicate-certificate limit.
(3) `cap_drop: [ALL]` together with `cap_add: [NET_BIND_SERVICE]`. cap_drop alone fails, because the binary carries a file capability.
(4) `security_opt: [no-new-privileges:true]`, `read_only: true` with `tmpfs: /tmp`, `mem_limit: 256m`.
(5) The tls-ask rules live in host-routing.
(6) README:
- Create the DNS record before adding a host.
- Remove hosts whose DNS no longer points at the server.
- `docker compose down -v` deletes certificates and all data.

No down/up CI assertion.

New dependency: none

Covers:

- Caddy, the internet-facing process holding every private key, gets no volumes or hardening, and tls-ask's load profile is unspecified
- Caddy, the only process exposed to the internet, gets none of Kohaku's hardening, and copying `cap_drop: [ALL]` onto it breaks it

### `backup-restore` — Repeatable backup, a restore command, an instance lock and graceful shutdown (low)

(1) `kohaku backup <file>` runs VACUUM INTO a temporary name in the target directory, fsyncs, renames, and sets mode 0600. It refuses an existing target with a clear message. The README uses a dated name under /data/backups, copies it off with `docker compose cp kohaku:/data/backups/<name> .`, then deletes it. With BLOB screenshots, that one file is the whole backup.
(2) `kohaku serve` holds an exclusive lock on /data/kohaku.lock using `std::fs::File::try_lock` (stable since Rust 1.89). This also prevents two instances.
(3) Add `kohaku restore <file>`, run as `docker compose run --rm kohaku kohaku restore …` while the service is stopped. It:
- refuses if the lock is held;
- runs `PRAGMA integrity_check` on the source;
- copies it into place as uid 65532 with mode 0600;
- removes -wal and -shm.
(4) Graceful shutdown on SIGTERM with axum `with_graceful_shutdown` (≤ 8 s), then close the DB connections.
(5) The smoke test runs backup and restore once.

Skip `backup -` to stdout.

New dependency: none

Covers:

- `kohaku backup` works only once, stores copies inside the live volume, and has no restore path; a naive restore corrupts the database

### `smoke-test` — Smoke test covers custom domains through the shipped Caddyfile and the release image (low)

(1) Add `kohaku project create <slug> --name <n> [--host <h>]`. Container access is already full access, so it grants nothing new.
(2) The shipped Caddyfile's global block contains `{$KOHAKU_CADDY_CI}`. CI sets it to `local_certs` plus `skip_install_trust`; in production it is empty. CI therefore tests the shipped Caddyfile.
(3) CI builds the image with the release Dockerfile from the build job's binaries. It creates one project with a host, copies root.crt out of the caddy-data volume, and uses `curl --cacert … --resolve` to assert:
- the main host serves a public page;
- the configured project host gets a certificate and serves `/`;
- an unknown host's TLS handshake fails;
- `/p/{slug}` answers 308.
(4) It also runs backup and restore once (see backup-restore).

No PoW-solving report POST in CI; integration tests cover submission.

New dependency: none

Covers:

- The Compose smoke test cannot reach custom domains or reports, and its local-CA override would test a different Caddyfile and a different image

### `install-from-tag` — Install files from the release tag; security lives in the image (low)

(1) The README's curl commands fetch compose.yaml, the Caddyfile and .env.example from `https://raw.githubusercontent.com/Dakes/Kohaku/X.Y.Z/…`, and the version is bumped at each release. No release-asset upload is needed in the job that holds secrets.
(2) Add a policy line to §10a: security-relevant behaviour (headers including HSTS, limits, validation) lives in the image, not in compose.yaml or the Caddyfile, because `docker compose pull` never updates those files.
(3) Release notes call out any compose or Caddyfile change.

New dependency: none

Covers:

- Installs download compose.yaml and the Caddyfile from `main`, which bypasses the release gate, and `docker compose pull` never updates them

### `libwebp` — Decide whether to keep the C libwebp encoder (low)

**Owner decision:** Option C: keep lossy WebP via libwebp, encoder only, decoder use forbidden.

Measure once on about 20 real screenshots (UI, game, photo-like). Compare libwebp lossy q80, image-webp lossless (already pulled in by image's webp decoder feature) and image's pure-Rust JPEG q80.

If the pure-Rust median size is within about 1.5× of libwebp:
- Drop the webp crate, libwebp-sys and the C compile from zigbuild and Nix.
- Update §2 and §3.

If libwebp stays:
- Keep it in one `encode_webp` module.
- Ban the decoder APIs with clippy `disallowed-types` and `disallowed-methods`.
- Record the vendored libwebp version in docs/dependencies.md.
- Reword §2 to 'untrusted encoded bytes never reach C'. Attacker-chosen pixel values still reach the encoder.

New dependency: none (option A/B removes webp and libwebp-sys)

Covers:

- The webp crate locks libwebp to a 2024 branch, compiles the whole C decoder, and exposes safe APIs that reach it

### `dependency-policy` — Enforce the dependency rules in cargo-deny and keep advisories flowing when the repo is quiet (low)

(1) deny.toml:
- `[bans] deny` = openssl, openssl-sys, native-tls, aws-lc-rs, aws-lc-sys, rayon, rav1e, ravif.
- `[bans.build] allow-build-scripts` = the build-script crates in the tree at adoption, so a new build script fails CI.
- `[sources] unknown-registry = "deny"` and `unknown-git = "deny"`.
(2) Remove cargo-audit from CI, the devShell and the AGENTS.md checklist. `cargo deny check advisories` covers the same RustSec database, and it becomes a gate in the release build job.
(3) Enable Dependabot alerts in the repo settings; they are not subject to the 60-day disable of scheduled workflows. Add .github/dependabot.yml for github-actions, docker and cargo, grouped and monthly. Policy: cut a patch release when an advisory affects a shipped crate.
(4) Describe lettre as 'rustls + ring + webpki-roots, no native-tls/aws-lc', not by feature names.
(5) Pin `idna_adapter = "~1.1"` in Cargo.toml to select the unicode-rs backend. This removes about 20 ICU4X crates, and IDN still works. Explain it in a comment and in docs/dependencies.md, and compare `cargo tree` before and after.

New dependency: idna_adapter (already transitive; a direct ~1.1 entry only selects the unicode-rs backend and removes about 20 ICU4X crates)

Covers:

- Advisories are only checked on push, and nothing updates the pinned things, so a quiet project keeps shipping known-vulnerable images
- The dependency choices depend on default-features = false, and the design's cargo-deny 'bans' has no content
- About 20 ICU4X crates come in through idna (lettre and url) and can be cut with one line

### `build-pinning` — Pin the toolchain, the Nix build, build tools and the base image (low)

(1) rust-toolchain.toml pins an exact version (`channel = "1.NN.P"`), bumped deliberately. It must be ≥ 1.89 for `File::try_lock`.
(2) `packages.default` uses `makeRustPlatform` with the rust-overlay toolchain from that file (`fromRustupToolchainFile`), not nixpkgs' rustc.
(3) Every cargo call in CI and release uses `--locked`. Install the build tool with `cargo install --locked cargo-zigbuild@=X.Y.Z`, and pin the zig version in CI.
(4) Pin the base image by digest (`gcr.io/distroless/static-debianNN:nonroot@sha256:…`) and keep it updated through Dependabot's docker ecosystem.
(5) CA source: webpki-roots compiled in through lettre's rustls. The image's CA bundle is unused; say so.
(6) Optional: run the aarch64 binary with `--version` on an ubuntu-24.04-arm runner before publishing.

No reproducible-build procedure.

New dependency: none

Covers:

- 'Same compiler everywhere' is not true as designed, and the release toolchain is not pinned or reproducible
- Floating base image and floating build infrastructure despite SHA-pinned actions; the CA source is undecided

### `nodejs-devshell` — Take OpenSpec from nixpkgs instead of nodejs plus npm (low)

**Owner decision:** Option A: pkgs.openspec from nixpkgs (1.13.1), no nodejs in devShell.

Replace `nodejs` in the devShell with `pkgs.openspec`. nixpkgs packages 1.13.1 with a pinned dependency hash; first check that the flake's nixpkgs revision includes it. flake.lock then pins it.

Non-Nix sandboxes install an exact version: `npm i -g --ignore-scripts @fission-ai/openspec@1.13.1`.

OpenSpec never runs in CI or release jobs.

**Applied as:** the owner's exact command, `npm i -g @fission-ai/openspec@1.13.1` (no `--ignore-scripts`).

New dependency: none

Covers:

- The devShell's nodejs brings back an unpinned npm supply chain on the machines that can push release tags

## Already resolved when reviewed

- **Reset/invite links built from the attacker-controlled Host header (reset poisoning with no click needed)**: The current §5 makes KOHAKU_BASE_URL required and says all absolute URLs (emails, redirects, canonical links) come from the URL builder using configured hosts only, never from the request Host. What was left is picked up elsewhere:
- Expected Origin from the configured hosts: origin-check.
- Forged Host / X-Forwarded-Host integration test: host-routing.
- Token links always on the main host: canonical-redirect.
- https-only BASE_URL validation: headers-and-cache.
- **Links in mails (password reset, invite, unsubscribe) have no fixed origin, so a forged Host header can steal reset tokens**: Resolved by the same §5 rule: a required KOHAKU_BASE_URL, and absolute URLs built only from configured hosts. Its List-Unsubscribe-Post one-click detail moved to outbox-content, and its GET-safe landing pages to token-flows.
- **Mail links (password reset, invite, unsubscribe) have no configured base URL, so an attacker can point them at their own host (Host / X-Forwarded-Host poisoning)**: Resolved by the required KOHAKU_BASE_URL and the configured-host URL builder in §5. The 'never read X-Forwarded-Host/Proto' rule is restated in client-ip-and-limiter-keys, and the CSRF expected origin in origin-check.

## Refuted by skeptics (round 1)

- **Public reads are unbounded: no pagination, and markdown is rendered on every view**: Refuted as an abuse issue:
- Only reports a maintainer has approved are public, so an attacker cannot grow the data set.
- List pages render titles, not bodies.
- Rendering a single report's markdown costs well under a millisecond.
- A read flood is volumetric and belongs at the reverse proxy, not in application design.

Pagination is ordinary hygiene, not a security finding.

The proposal to store pre-rendered HTML has a security downside: a later ammonia or renderer fix would not reach HTML already stored unless everything is re-rendered.
- **Disabled or de-granted maintainers keep getting private pending-report mail, and their outstanding tokens stay valid**: This is overstated. Coalesced notifications are composed at flush time (§7, ≤ 1 per 10 min), which naturally resolves current membership and opt-outs. Only mail already composed before the de-grant can still arrive during retries, and that content was visible to the maintainer when it was composed. The links in those mails need a session and return 404 to a non-member (§4), so at most a few titles leak. The part about a disabled user resetting and logging in back in only matters if reset creates a session, which the reset finding already fixes ('reset never creates a session; reset for disabled users does nothing'). Login rejecting disabled users is implied by the `disabled` column.
- **Unmoderated report titles go into maintainer notification mails (phishing from the real sender, and a truncation panic)**: This is mostly already covered. The AGENTS.md 'New email' checklist says 'User-supplied text in a subject or body must be single-line, length-capped, and marked as quoted', and §7 strips CR/LF. That handles layout forgery and header injection.

The phishing scenario is inherent to any issue tracker's notification mail: GitHub mails carry issue text from anonymous users. Maintainers are exactly the people whose job is to read unmoderated reports, and the admin UI shows them the same text anyway. Removing titles from notifications would cost most of the triage value for little security gain.

The other two points are generic Rust hygiene, not design flaws:
- A byte-slice truncation panic.
- Panic isolation in the outbox worker.

The one small, concrete addition is rejecting bidi override and invisible format characters in titles. That is a detail of the validation rule, not a finding.
- **Nothing stops a template from using askama's `|safe` or placing values in URL contexts**: The design already covers this. The AGENTS.md checklist for new public input says 'Rendered only through askama escaping or the Markdown sanitizer, never `|safe` on user data.'

I confirmed the technical claims:
- askama 0.16.1 exports `filters::HtmlSafe`.
- It auto-escapes only these extensions: askama, html, htm, j2, jinja, jinja2, rinja, svg and xml.

The remaining advice is a generic best practice: never interpolate into script, style or on* attributes. The finding describes no concrete attack beyond a future coding slip.

The newtype is a good zero-dependency way to enforce the existing rule, so it is worth adopting when the markdown change is implemented. It is not a design defect.
- **Request logs and error responses can leak tokens and internals**: The main points are already covered. §11 says logs never contain tokens, emails or IPs, and AGENTS.md adds 'A test asserts this', which catches a TraceLayer span that records the full URI. AGENTS.md's 'No state changes on GET' already requires invite, reset and unsubscribe to be confirmation forms that POST, so link scanners can't burn tokens. The rest is generic hardening with negligible impact on an open-source app. SQL and constraint names are public in the repo, and serde errors only echo the requester's own input back to them, askama-escaped. Caddy does not write access logs by default.
- **SMTP: the dev setup invites a plaintext / skip-certificate-check option into production, and secrets are not redacted**: This is speculative, and the design already covers it. §7 says 'TLS required', §3 says 'lettre (rustls only)' and that every secret has a `*_FILE` variant, and AGENTS.md forbids workarounds and fallbacks and requires default features off. Nothing in the design proposes a plaintext or skip-verification option. Redacting secrets in Debug output falls under the existing logging rule. The only real point is a planning detail: the §8 Mailpit dev setup has to fit 'TLS required'.
- **DB, WAL, backup, media and secret files are created readable by all local users; /data ownership pushes operators to weaken the container**: This does not apply to the first-class deployment. §10a uses a named volume, which lives under Docker's root-only volume directory, and the container runs as a single non-root user with no other local accounts. A Nix/systemd install normally gets a 0700 StateDirectory. The exposure needs a bind mount on a multi-user host with a permissive parent directory, which is generic hardening. The chmod-777 temptation also only arises with bind mounts, which are not the documented path.
- **Read-only container blocks SQLite temp files; the healthcheck mechanism and /healthz content are unspecified**: The current design already covers this. §10a mounts `tmpfs: /tmp`, and SQLite uses the first writable temp directory, so it falls back to /tmp. §3 already defines a `kohaku healthcheck` subcommand for the distroless image. That leaves the /healthz body, and leaking a version string for an open-source app has negligible value to an attacker.
