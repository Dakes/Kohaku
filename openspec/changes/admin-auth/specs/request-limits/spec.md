# Spec Delta

## MODIFIED Requirements

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

## ADDED Requirements

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
