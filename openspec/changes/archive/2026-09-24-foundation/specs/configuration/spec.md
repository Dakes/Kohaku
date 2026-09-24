# Spec Delta

## Purpose

How an operator configures Kohaku (environment variables only, secrets included), how settings,
the instance secret and keys are validated and held, and when Kohaku refuses to start (design
baseline §3 Config, §5 Key inventory).

## ADDED Requirements

### Requirement: Configuration sources

Kohaku SHALL take configuration, the secrets `KOHAKU_SECRET` and `KOHAKU_SMTP_PASSWORD`
included, only from environment variables, read once when a command starts: no configuration
file of its own (TOML or other), no setting or secret from a command-line argument (operations)
and no HTTP route on any host that displays, returns or changes a setting.

#### Scenario: A configuration file is ignored
- **WHEN** a `kohaku.toml` naming another base URL and another secret exists in the working directory and `/data`
- **THEN** `kohaku serve` uses `KOHAKU_BASE_URL` and `KOHAKU_SECRET`

### Requirement: Settings and defaults

Kohaku SHALL use these settings, each required with no default unless marked optional; a
required variable set to the empty string SHALL count as missing:

- `KOHAKU_BASE_URL` (main-host origin, used as host-routing specifies), `KOHAKU_TRUSTED_PROXIES`,
  `KOHAKU_SECRET`, `KOHAKU_SMTP_HOST`, `KOHAKU_SMTP_PORT`, `KOHAKU_SMTP_TLS`,
  `KOHAKU_SMTP_USERNAME`, `KOHAKU_SMTP_PASSWORD`, `KOHAKU_SMTP_FROM` (sender of every mail).
- `KOHAKU_PUBLIC_MAIL_PER_HOUR`, optional: the instance-wide number of unauthenticated mails per
  hour, one budget shared by every public mail trigger and enforced as request-limits specifies;
  60 when unset, else a decimal 1–4294967295 without sign, spaces or leading zeros (`0` and the
  empty string are rejected, not treated as unset).
- A `dev` build sends no mail (mail-outbox), so it SHALL NOT read `KOHAKU_SMTP_HOST`,
  `KOHAKU_SMTP_PORT`, `KOHAKU_SMTP_TLS`, `KOHAKU_SMTP_USERNAME` or `KOHAKU_SMTP_PASSWORD`, and
  SHALL refuse `kohaku serve` while any of them is set, even empty, naming each;
  `KOHAKU_SMTP_FROM` stays required.
- Rate-limit budgets, concurrency bounds, body caps and request deadlines SHALL be constants. No
  setting SHALL disable mail TLS or certificate verification, add a trusted certificate, or allow
  an `http` base URL in a release build.

#### Scenario: Public mail budget values
- **WHEN** `KOHAKU_PUBLIC_MAIL_PER_HOUR` is unset, `120`, or any of `0`, `-1`, `060`, `60/h`, `4294967296` or the empty string
- **THEN** the budget is 60 or 120 per hour respectively; each other value makes `kohaku serve` exit non-zero naming the variable

#### Scenario: Development build takes no SMTP server
- **WHEN** a `dev` build's `kohaku serve` starts with `KOHAKU_SMTP_FROM` and no other SMTP setting, and again with `KOHAKU_SMTP_HOST` also set to the empty string
- **THEN** the first starts; the second exits non-zero naming `KOHAKU_SMTP_HOST`

### Requirement: Startup validation and error reporting

`kohaku serve` SHALL validate every setting and decode the instance secret before it creates or
opens anything in the data directory or listens on any port. On invalid configuration it SHALL
report every problem of the run, each naming the variable and the broken rule, then exit
non-zero.

- Values SHALL be used exactly as given, never trimmed, case-folded or repaired; a value that is
  not valid UTF-8 is invalid. No error or log line SHALL contain a secret's value, valid or not.
- Kohaku SHALL refuse to start while `KOHAKU_BASE_URL`, `KOHAKU_SMTP_HOST`, `KOHAKU_SMTP_USERNAME`
  or `KOHAKU_SMTP_FROM` equals its shipped `.env.example` placeholder (the base URL as
  `docker-compose.yml` builds it from the `KOHAKU_DOMAIN` placeholder), naming each; the check SHALL
  cover every setting `.env.example` fills with a placeholder instead of a working value.
- `kohaku backup` SHALL require and validate only `KOHAKU_SECRET`; `kohaku restore` and
  `kohaku healthcheck` SHALL require no setting from this spec. Any setting a command reads SHALL
  be validated by the `kohaku serve` rules.

#### Scenario: Every problem is reported in one run
- **WHEN** `kohaku serve` starts on an empty data directory with `KOHAKU_TRUSTED_PROXIES` unset, `KOHAKU_SMTP_FROM` empty and `KOHAKU_SMTP_PORT` set to ` 587`
- **THEN** it exits non-zero naming all three (the first two as missing, no default assumed), the data directory stays empty and nothing listens on port 8080

#### Scenario: Unchanged example configuration
- **WHEN** `kohaku serve` starts with both secrets set and otherwise an unchanged copy of the shipped `.env.example` and `docker-compose.yml`, or with only `KOHAKU_SMTP_FROM` still at its placeholder
- **THEN** it exits non-zero naming as unchanged example values all four settings, or only `KOHAKU_SMTP_FROM`

#### Scenario: Commands need only their own settings
- **WHEN** no setting from this spec is present and `kohaku healthcheck`, `kohaku restore <file>` and `kohaku backup -` run, the backup once more with a valid matching `KOHAKU_SECRET`
- **THEN** only the first backup fails for a missing setting, naming `KOHAKU_SECRET` and writing nothing; the second writes the backup

### Requirement: Instance secret

`KOHAKU_SECRET` SHALL hold the instance secret as base64 in the standard alphabet with `=`
padding (RFC 4648 §4), without whitespace or line breaks, decoding to at least 32 bytes; any other
value SHALL be rejected with an error stating that format and the 32-byte minimum. Kohaku SHALL
NOT write the secret, its base64 text or any key equal to it to the database, any data-directory
file (backups included), a log line or a response. Every value derived from it SHALL be
HMAC-SHA256 keyed with the decoded secret over an input starting with a purpose label unique to
that use, every part (label included) length-prefixed; MACs under per-boot keys SHALL use the
same encoding, and every MAC check SHALL compare in constant time. The only derived value in
this change is the keycheck (`kohaku/keycheck`); a later change deriving another SHALL give it
its own label.

#### Scenario: Secret encoding
- **WHEN** `KOHAKU_SECRET` holds the output of `head -c 32 /dev/urandom | base64` without its final LF, or instead with that LF or a leading space, base64 of 31 bytes, of 32 bytes URL-safe or without `=` padding, or of 64 bytes wrapped over two lines
- **THEN** the first is accepted as 32 secret bytes; each other makes `kohaku serve` exit non-zero naming `KOHAKU_SECRET`, the format and the 32-byte minimum, without the value

#### Scenario: MAC inputs cannot collide
- **WHEN** MACs under one key are computed for label `kohaku/x` with fields `ab`,`c` and `a`,`bc`, and for `kohaku/a` with `bc` and `kohaku/ab` with `c`
- **THEN** the MACs of each pair differ

### Requirement: Database keycheck

The database SHALL hold one keycheck, the value derived with label `kohaku/keycheck`. On a newly
created database without one, `kohaku serve` SHALL store the configured secret's keycheck before
accepting any request, and every other command except `kohaku restore` SHALL refuse the
database. Otherwise every command that opens the database except `kohaku restore` SHALL compare
the stored keycheck with the configured secret's before applying a migration, copying the
database, writing any row or listening on any port; on a mismatch it SHALL exit non-zero stating
that `KOHAKU_SECRET` does not match the database, with neither the secret nor either
keycheck in the error. `kohaku restore`, the only command in this change that skips the
keycheck, SHALL NOT compare it; the next command opening the restored database does.

#### Scenario: Wrong secret refused
- **WHEN** `kohaku serve` starts on an empty data directory with secret A, restarts with A, then starts with secret B
- **THEN** both A starts serve requests; the B start exits non-zero with the mismatch error, schema version and rows unchanged, no file added to `/data/backups`, nothing listening on port 8080

#### Scenario: Backup refused with the wrong secret
- **WHEN** `kohaku backup <file>` runs on a database created with secret A while `KOHAKU_SECRET` holds secret B
- **THEN** it exits non-zero with the mismatch error and `<file>` is not created

#### Scenario: Restore skips the keycheck
- **WHEN** `kohaku restore <file>` restores a backup made under secret A while `KOHAKU_SECRET` holds B or is unset
- **THEN** it completes; the next `kohaku serve` exits with the mismatch error under B and starts under A

### Requirement: Per-boot keys

Every per-boot key SHALL be 32 bytes from the operating system's cryptographically secure random
source, held only in process memory, not derived from the instance secret, never written to the
database, a file, a log line or a response; a random-source failure SHALL yield an error, not a
key. `kohaku serve` SHALL generate its per-boot keys at start (a restart replaces each) and exit
non-zero instead of starting if generation fails; these two rules apply from the first change
that creates one in `serve`.

#### Scenario: Attacker obtains the data directory and logs
- **WHEN** two per-boot keys are generated, and an attacker obtains the database, its WAL, a `kohaku backup` output, every other data-directory file and the log output while `kohaku serve` runs
- **THEN** the keys differ and are 32 bytes each, and nothing obtained contains them, any per-boot key of that process, or the instance secret's bytes or base64 text

### Requirement: Base URL format

`KOHAKU_BASE_URL` SHALL be exactly `https://`, a host and an optional `:port`, with no path (not
even `/`), query, fragment or user information. The host SHALL be a lowercase ASCII DNS name:
labels of 1–63 characters from `a`–`z`, `0`–`9` and `-`, none starting or ending with `-`, at
most 253 characters, no trailing dot, internationalized names in punycode (`xn--`), no IPv4 or
IPv6 literal. The port SHALL be decimal 1–65535 without leading zeros and not 443. Only a `dev`
build SHALL also accept `http://localhost` with an optional port other than 80.

#### Scenario: Base URL values
- **WHEN** it is `https://bugs.kohaku.test`, `https://bugs.kohaku.test:8443` or (`dev` build) `http://localhost:8080`; or `https://bugs.kohaku.test/`, `https://bugs.kohaku.test/kohaku`, `https://bugs.kohaku.test?x=1`, `https://bugs.kohaku.test#x`, `https://user@bugs.kohaku.test`, `HTTPS://bugs.kohaku.test`, `https://Bugs.Kohaku.test`, `https://bugs.kohaku.test.`, `https://bugs.kohaku.test:443`, `https://bugs.kohaku.test:0443`, `https://bugs.kohaku.test:0`, `https://192.0.2.10`, `https://[2001:db8::1]`, `http://bugs.kohaku.test`, `http://127.0.0.1:8080` or (release build) `http://localhost:8080`
- **THEN** the first three are accepted; each other makes `kohaku serve` exit non-zero naming `KOHAKU_BASE_URL`, and for a path, query, fragment or user information the form `https://host[:port]`

### Requirement: Trusted proxy list format

`KOHAKU_TRUSTED_PROXIES` SHALL be `none` alone (no peer is a trusted proxy; request-limits
specifies how trust is used) or a comma-separated list of one or more entries without spaces or
empty entries, each an exact address (IPv4 dotted-decimal, four parts 0–255 without leading
zeros; or IPv6 in RFC 4291 text form without zone index) or a CIDR block `address/length` (1–32
for IPv4, 1–128 for IPv6, no leading zeros) with no bits set beyond the prefix, trusting every
address in it. IPv4-mapped IPv6 addresses and blocks (inside `::ffff:0:0/96`) and prefix length
0 SHALL be rejected. Documentation SHALL give exact addresses as the normal form and CIDR blocks
as a last resort.

#### Scenario: Accepted and rejected lists
- **WHEN** it is `10.231.7.2,fd4b:7a1c:2e90:1::2` (shipped Compose), `none`, `10.231.7.0/29` or `fd4b:7a1c:2e90:1::/64`; or any of `10.231.7.2, fd00::2`, `10.231.7.2,,10.231.7.3`, `010.231.7.2`, `fe80::1%eth0`, `::ffff:10.231.7.2`, `caddy`, `none,10.231.7.2`, `NONE`, `10.231.7.2/24`, `0.0.0.0/0`, `::/0`
- **THEN** the first four are accepted (the first trusting exactly its two addresses); each other makes `kohaku serve` exit non-zero naming `KOHAKU_TRUSTED_PROXIES`

#### Scenario: Attacker sends a forwarded-for header with no trusted proxy
- **WHEN** `KOHAKU_TRUSTED_PROXIES` is `none` and an attacker connecting from `203.0.113.5` sends `X-Forwarded-For: 198.51.100.7`
- **THEN** Kohaku treats `203.0.113.5` as the client address

### Requirement: SMTP settings

The SMTP settings a build reads SHALL be validated as below without contacting the server; an
unreachable server or rejected credentials SHALL NOT stop `kohaku serve` from starting (failures
are mail-outbox's).

- `KOHAKU_SMTP_HOST`: a DNS name under the base-URL host rules, no scheme, port, path or IP
  literal; the server certificate is verified against it.
- `KOHAKU_SMTP_PORT`: decimal 1–65535 without sign or leading zeros.
- `KOHAKU_SMTP_TLS`: `implicit` (TLS from the first byte) or `starttls` (upgrade before
  authentication and any mail data; a server not offering STARTTLS fails the delivery attempt
  instead of receiving plaintext); nothing else.
- `KOHAKU_SMTP_USERNAME` and `KOHAKU_SMTP_PASSWORD`: no control characters.
- `KOHAKU_SMTP_FROM`: one `local@domain` of at most 254 characters with exactly one `@`, a
  non-empty local part, a domain under the host rules, and no whitespace, control characters,
  display name or angle brackets.

#### Scenario: Malformed SMTP values rejected
- **WHEN** `KOHAKU_SMTP_TLS` is `none`, `off`, `STARTTLS` or empty, `KOHAKU_SMTP_PORT` is `0`, `65536` or `smtp`, `KOHAKU_SMTP_HOST` is `smtp://smtp.kohaku.test` or `smtp.kohaku.test:587`, or `KOHAKU_SMTP_FROM` is `Kohaku <kohaku@bugs.kohaku.test>` or `a@b@bugs.kohaku.test`
- **THEN** `kohaku serve` exits non-zero naming each offending variable, for `KOHAKU_SMTP_TLS` with the accepted values `implicit` and `starttls`

#### Scenario: Unreachable SMTP server does not block startup
- **WHEN** `kohaku serve` starts with `smtp.kohaku.test`, `587`, `starttls`, `kohaku`, the password `correct horse battery staple` and `kohaku@bugs.kohaku.test` while nothing listens there
- **THEN** it accepts them, starts and serves requests

#### Scenario: Attacker strips STARTTLS
- **WHEN** `KOHAKU_SMTP_TLS` is `starttls` and an on-path attacker removes the STARTTLS offer from the server's greeting
- **THEN** Kohaku sends neither the SMTP credentials nor any mail data over that connection
