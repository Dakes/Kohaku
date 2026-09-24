# request-limits Specification

## Purpose
Client-address resolution behind trusted reverse proxies, per-network token-bucket rate
limits, the public mail budget and the public write concurrency bound (design baseline §3
Client IP, Rate limiter, Concurrency and memory bounds).

## Requirements

### Requirement: Client address resolution scope

Only requests that reach the host map (host-routing) SHALL resolve a client address.
`/healthz` and `/.well-known/kohaku/tls-ask`, matched before it for any `Host`, SHALL NOT
resolve one, log a client-address warning or draw from any rate-limit or mail bucket. Every
IP-keyed limit SHALL use the one resolved address and never read forwarding headers itself.

#### Scenario: Attacker floods an internal endpoint
- **WHEN** `kohaku healthcheck` probes `/healthz` from `127.0.0.1`, trusted proxy `10.231.7.2` requests `/.well-known/kohaku/tls-ask?domain=bugs.example.com` with `Host: kohaku:8080` and no `X-Forwarded-For`, and an attacker at `203.0.113.5` sends 1000 `/healthz` requests, then 300 `GET /` on the main host
- **THEN** no client-address warning is logged, no `/healthz` request gets 429 and all 300 `GET /` get 200

### Requirement: Trusted proxies and the forwarded-for header

The client address SHALL be derived only from the TCP peer and `X-Forwarded-For`, never from
`Forwarded`, `X-Real-IP`, `X-Forwarded-Host` or `X-Forwarded-Proto`:

- The peer and every entry SHALL be canonicalized (`::ffff:a.b.c.d` is IPv4 `a.b.c.d`)
  before trust comparison, prefix masking or bucket keying.
- A peer is trusted when it equals an exact address or lies in a CIDR range of
  `KOHAKU_TRUSTED_PROXIES` (configuration); with `none`, no peer is.
- An untrusted peer's header is ignored; for it, and for a trusted peer without the header,
  the client is the peer.
- Otherwise all header lines are joined in received order into one list and walked right to
  left, skipping trusted entries; the client is the first untrusted entry, or the leftmost if
  all are trusted.
- Every entry, trimmed of spaces and tabs, SHALL be a bare IPv4 or IPv6 literal. One empty
  entry, port, bracket, zone index, hostname, `unknown` or byte outside visible ASCII makes
  the whole header ignored and the client the peer.

#### Scenario: Attacker forges the header without a trusted peer
- **WHEN** `KOHAKU_TRUSTED_PROXIES` is `10.231.7.2` and an attacker at `203.0.113.5` sends `X-Forwarded-For: 198.51.100.7`, or it is `none` and peer `10.231.7.2` sends that header, or trusted peer `10.231.7.2` sends no `X-Forwarded-For`
- **THEN** the client address is the peer (`203.0.113.5`, `10.231.7.2`, `10.231.7.2`)

#### Scenario: Attacker forges entries in front of the proxy's
- **WHEN** `KOHAKU_TRUSTED_PROXIES` is `10.231.7.2` and that peer forwards `X-Forwarded-For: 192.0.2.1, 198.51.100.7`, `X-Forwarded-For: 10.231.7.2, 198.51.100.7`, or an attacker's line `X-Forwarded-For: 192.0.2.1` followed by its own line `X-Forwarded-For: 198.51.100.7`
- **THEN** the client address is `198.51.100.7` each time

#### Scenario: Attacker sends other forwarding headers through a chain of trusted proxies
- **WHEN** `KOHAKU_TRUSTED_PROXIES` is `10.231.7.0/29` and mapped peer `::ffff:10.231.7.2` forwards `Forwarded: for=192.0.2.1`, `X-Real-IP: 192.0.2.1` and `X-Forwarded-For: 198.51.100.7, 10.231.7.3`, then only `X-Forwarded-For: 10.231.7.3`
- **THEN** the client address is `198.51.100.7`, then `10.231.7.3`

#### Scenario: Attacker injects a malformed entry
- **WHEN** `KOHAKU_TRUSTED_PROXIES` is `10.231.7.2` and that peer forwards `X-Forwarded-For: unknown, 198.51.100.7`, `192.0.2.1,,198.51.100.7`, `198.51.100.7:4711` or `192.0.2.1 ,<TAB>198.51.100.7 `
- **THEN** the client address is `10.231.7.2` for the first three and `198.51.100.7` for the last

### Requirement: One-time client-address warnings

Each warning below SHALL be logged at most once per server start, when its condition first
occurs. It SHALL contain no IP address (peer, entry or resolved address) and SHALL NOT change
the resolved address or the response.

- `XFF from untrusted peer`: an untrusted peer sent `X-Forwarded-For`.
- `trusted peer sent no XFF`: a trusted peer sent none.
- `client address is loopback/RFC 1918/ULA/link-local`: the resolved address lies in
  `127.0.0.0/8`, `::1/128`, `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `fc00::/7`,
  `169.254.0.0/16` or `fe80::/10`.

#### Scenario: Attacker repeats a forged header
- **WHEN** `KOHAKU_TRUSTED_PROXIES` is `10.231.7.2` and an attacker at `203.0.113.5` sends 1000 requests with `X-Forwarded-For: 192.0.2.1`
- **THEN** `XFF from untrusted peer` is logged exactly once and no log line contains `203.0.113.5` or `192.0.2.1`

#### Scenario: Proxy problems are flagged once, public clients never
- **WHEN** trusted peer `10.231.7.2` forwards a main-host request with `X-Forwarded-For: 198.51.100.7`, then two with `X-Forwarded-For: 172.18.0.1`, then two without the header
- **THEN** the first logs no warning, the next two log `client address is loopback/RFC 1918/ULA/link-local` once and the last two `trusted peer sent no XFF` once, all without any address

### Requirement: Rate-limit keys by network prefix

Each rate-limit class SHALL keep its own token buckets, keyed by the resolved client address:

- IPv4 per /32, with no aggregate; IPv6 per /64 plus a /48 aggregate holding 8 times the
  per-/64 budget.
- A request SHALL be admitted only when every bucket of its key holds a token, and then take
  one from each; a rejected request takes none.
- A new bucket SHALL start full and refill continuously at budget / window, capped at its
  budget and never reset at a window boundary (`read`: one token per 200 ms per /32 or /64,
  per 25 ms per /48).

#### Scenario: Attacker rotates addresses and /64s inside one /48
- **WHEN** an attacker sends in one burst 1300 read requests from distinct addresses inside `2001:db8:1:1::/64`, then 300 from each of 7 other /64s inside `2001:db8:1::/48`, then one from a ninth /64 there
- **THEN** 300 of the first 1300 and all 2100 from the 7 /64s are admitted, the ninth /64's request gets 429, and a read request from `2001:db8:2::1` is admitted

#### Scenario: Attacker on IPv4 cannot exhaust its neighbours
- **WHEN** `KOHAKU_TRUSTED_PROXIES` is `none` and an attacker at peer `::ffff:198.51.100.7` sends 300 read requests in one burst
- **THEN** its next request, from peer `198.51.100.7`, gets 429, and a read request from peer `::ffff:198.51.100.8` is admitted

#### Scenario: Refill is even and cannot be saved up
- **WHEN** an IPv4 client that emptied its read bucket waits 200 ms, and an attacker at `203.0.113.5` idles for 10 minutes after its first request, then each sends a burst
- **THEN** exactly one of the client's requests is admitted before a 429, and at most 300 of the attacker's 1000

### Requirement: Read rate-limit class

The `read` class SHALL have budgets fixed in the binary (not configurable): 300 per minute per IPv4 /32 or IPv6 /64 and 2400 per minute
per IPv6 /48, shared across all hosts. It SHALL cover every GET and HEAD to a route of the
main-host or project-host router, signed-in admin pages and the `/admin` redirects included,
as well as their 404 fallbacks, and SHALL NOT cover `/static/` paths, internal endpoints or 421 unknown-host
requests. Per the http-security rejection order, a GET or HEAD rejected by admin resource
isolation SHALL NOT draw a token, and one answered 415 by the multipart rule SHALL.

#### Scenario: Attacker scans unknown paths
- **WHEN** an attacker sends 150 `HEAD /`, then 150 GETs for distinct nonexistent paths, then one `GET /` to the main host in one burst, then requests an existing `/static/` asset
- **THEN** the HEADs get 200, the GETs 404, the last `GET /` 429 and the asset 200

#### Scenario: Attacker sends a multipart GET
- **WHEN** an attacker sends `GET /` with `Content-Type: multipart/form-data; boundary=x` to the main host, once with read tokens left and once with an empty read bucket
- **THEN** the first gets 415 and takes one read token, the second 429

### Requirement: Rate-limited requests are rejected before any work

A request rejected by a rate-limit class SHALL be answered 429 Too Many Requests before its
body is read, the database is accessed or its handler runs. Neither the 429 response nor any log line
for it SHALL contain the client address or the bucket key.

#### Scenario: Attacker keeps hammering after exhaustion
- **WHEN** an attacker at `203.0.113.5` has emptied its read bucket and sends 1000 more read requests in the same burst to a route whose handler counts its invocations
- **THEN** all 1000 get 429, the handler is never invoked, and no response header, body or log line contains `203.0.113.5`

### Requirement: Bounded limiter memory

The limiter SHALL hold at most 100 000 buckets, counting every class, /48 aggregates and
per-source mail buckets. A request needing a missing bucket at the cap SHALL take at most one
token from an overflow bucket instead of creating one; existing buckets keep their own
tokens. Each class and the per-source mail budget SHALL have exactly one shared overflow
bucket with its per-key budget (`read` 300 per minute, per-source mail 10 per hour). Every
60 s the limiter SHALL remove every bucket refilled to its full budget and keep every other.

#### Scenario: Attacker floods with fresh prefixes
- **WHEN** a client at `198.51.100.7` holds a full read bucket and an attacker sends one read request from each of 150 000 distinct IPv6 /48s in one burst
- **THEN** the limiter holds at most 100 000 buckets, at most 300 attacker requests without a bucket of their own are admitted (the rest get 429), and a read request from `198.51.100.7` is admitted

#### Scenario: Attacker cannot use the sweep to reset its bucket
- **WHEN** the `read` overflow bucket is empty, the limiter holds 100 000 read buckets idle for 120 s except that of an attacker at `203.0.113.5`, and a sweep runs 10 s after the attacker emptied it
- **THEN** the sweep removes every idle bucket and keeps the attacker's; a new client's read request is admitted, and at most 50 of an attacker burst sent then

### Requirement: Limiter state lives in memory only

Every rate-limit and public mail bucket SHALL live in process memory only: bucket state,
bucket keys and client addresses SHALL NOT be written to the database, any file under `/data`
or the logs. After a restart every bucket SHALL start full (intended).

#### Scenario: Attacker with a leaked backup learns no client addresses
- **WHEN** clients at `198.51.100.7` and `2001:db8:1:2::5` have emptied their read buckets, an attacker obtains a later `kohaku backup`, and the server restarts
- **THEN** the backup contains neither address nor any rate-limit state, and each client's next 300 read requests are admitted

### Requirement: Public mail budget

Each unauthenticated request that causes mail (the OTP send and password reset request
routes of later changes, which define where this check sits) SHALL, besides its route's
rate-limit class, take one token from its source's bucket (IPv4 /32 or IPv6 /48: 10 per
hour, one regained every 6 min) and one from the instance-wide bucket
(`KOHAKU_PUBLIC_MAIL_PER_HOUR` per hour, default 60, validated by configuration: one regained
every 3600 s / that value). If either is empty it SHALL take none, queue no mail and get 429
"try again later", identical for either bucket and without the client address.

#### Scenario: Attacker from one network
- **WHEN** an attacker sends 11 mail-triggering requests in one burst, from `203.0.113.5` or from 11 different /64s inside `2001:db8:1::/48`
- **THEN** 10 are admitted, the 11th gets 429 with no mail queued, and the instance-wide bucket has lost exactly 10 tokens

#### Scenario: Distributed attacker hits the instance-wide cap
- **WHEN** `KOHAKU_PUBLIC_MAIL_PER_HOUR` is unset and an attacker sends one mail-triggering request from each of 61 distinct IPv6 /48s in one burst
- **THEN** 60 are admitted; the 61st gets 429 with no mail queued and the same status, headers other than date, and body as a rejection for an empty source bucket

#### Scenario: Configured budget and refill
- **WHEN** `KOHAKU_PUBLIC_MAIL_PER_HOUR` is `120`, 13 distinct /32s send 10 mail-triggering requests each in one burst, and 30 s later two more arrive from a source with tokens left
- **THEN** exactly 120 of the burst are admitted, then exactly one of the two

### Requirement: Bounded public write concurrency

At most 8 database writes for unauthenticated public requests (report submission, OTP and
+1, added by later changes) SHALL run at once. A public write finding all 8 permits taken
SHALL get 503 Service Unavailable at once, without waiting or writing anything. A permit SHALL
return as soon as its write ends: success, error, request deadline or client disconnect.
Authenticated-request, CLI and background-job writes SHALL NOT take or wait for a permit.

#### Scenario: Attacker floods public writes
- **WHEN** an attacker starts 20 public writes at once while each is held in progress
- **THEN** at most 8 run and the other 12 get 503 immediately with nothing written for them

#### Scenario: Only public writes need permits, and failures return them
- **WHEN** all 8 permits are held, the retention job writes, then one held write fails and a new public write starts
- **THEN** the retention job's writes complete without waiting or 503, and the new write gets a permit

### Requirement: Login and reset rate-limit classes

Two more classes SHALL use the per-prefix buckets of "Rate-limit keys by network prefix",
with budgets fixed in the binary:

- `login`: 10 per 15 minutes per IPv4 /32 or IPv6 /64, 80 per 15 minutes per IPv6 /48. It
  SHALL cover every request that checks a password, a TOTP code or recovery code, or a
  single-use account token: `POST /admin/login`, `POST /admin/reset/{token}`, and every
  re-authenticated account change (admin-auth).
- `reset`: 5 per hour per IPv4 /32 or IPv6 /64, 40 per hour per IPv6 /48, covering
  `POST /admin/reset`.

A request SHALL draw only from its own class; `read` never covers these POSTs.

#### Scenario: Attacker guesses passwords from one network
- **WHEN** an attacker at `203.0.113.5` sends 11 login POSTs within one minute for 11 different accounts
- **THEN** 10 are processed and the 11th gets 429 without its body being read or a password hashed

#### Scenario: Attacker spreads over an IPv6 /48
- **WHEN** an attacker sends 10 login POSTs from each of 9 different /64s inside `2001:db8:5::/48` within 15 minutes
- **THEN** 80 are processed and the rest get 429

### Requirement: Per-address reset limit

Each reset request that passes its `reset` token and has a valid email address SHALL take a
token from the bucket of its normalized address: ASCII-lowercased after removing surrounding
ASCII whitespace, with any `+tag` removed from the local part, no provider-specific rules. The
bucket SHALL allow 3 per hour and refill one every 20 minutes. Its key SHALL be an HMAC of
the normalized address under a per-boot key (label `kohaku/mail-address`), held in the
limiter's memory only, counting toward the 100 000-bucket cap with its own overflow bucket.
Then the public mail budget applies. An empty bucket SHALL get the same 429 "try again later"
as the public mail budget and queue nothing.

#### Scenario: Attacker mail-bombs one mailbox from many networks
- **WHEN** an attacker requests resets for `victim@example.org`, `Victim+a@Example.org`, ` victim+b@example.org` and `VICTIM@example.org` from four different /48s within one hour
- **THEN** the first three are accepted and the fourth gets 429, and no log line or database row contains the address or its HMAC
