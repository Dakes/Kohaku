# operations Specification

## Purpose
How operators run and observe Kohaku: the `kohaku` command line, the health endpoint and
healthcheck, graceful shutdown and what logs may contain (design baseline §3 "CLI", §10).

## Requirements

### Requirement: Command-line commands

The `kohaku` binary SHALL accept exactly `serve`, `healthcheck`, `backup <file>`, `backup -`,
`restore <file>`, `restore -`, `restore --list`, `admin unlock --email <address>`,
`admin reset-password --email <address>`, `--version`, `--help` and `<command> --help`
(`admin unlock --help` and `admin reset-password --help` for the two-word commands), names
and flags matched exactly, case included.

- `--version` SHALL print `kohaku <version>\n` to stdout, `<version>` being the built version,
  equal to the release tag without `v` (e.g. `1.0.0`); `--help` and `<command> --help` SHALL
  print the usage text, naming every invocation, to stdout and run nothing. Both exit 0 with
  no `KOHAKU_` variable, `/data` or network, and create or modify no file.
- Every command SHALL exit 0 on success, 2 on a usage error and 1 on any other failure, whose
  cause it first reports on its log stream.
- Stdout SHALL carry only data (the `backup -` backup, the `restore --list` listing, the
  `admin reset-password` link, the `--version` and `--help` text), so `admin unlock`, `healthcheck`, `backup <file>`, `restore <file>` and
  `restore -` write nothing there. Every command but `serve` SHALL log (warnings and errors
  included) to stderr only; `serve`, which has no data output, SHALL log to stdout.

#### Scenario: Version and help need no configuration

- **WHEN** `kohaku --version`, `kohaku --help` or `kohaku backup --help` runs with no `KOHAKU_` variable and no `/data`
- **THEN** each exits 0 and creates no file; `--version` prints exactly `kohaku <version>\n` with empty stderr; the help names `serve`, `healthcheck`, `backup`, `restore`, `restore --list`, `admin unlock` and `admin reset-password`

#### Scenario: A refusal is a failure, not a usage error

- **WHEN** `kohaku backup <file>` names an existing file, or `kohaku restore <file>` runs while `kohaku serve` holds the instance lock
- **THEN** the exit status is 1, stderr names the cause and stdout is empty

#### Scenario: Malformed admin invocations

- **WHEN** `kohaku admin`, `kohaku admin unlock`, `kohaku admin unlock --email`, `kohaku admin unlock --email a@b.test extra`, `kohaku admin unlock a@b.test` or `kohaku admin frobnicate` runs
- **THEN** each prints the usage text to stderr and exits 2 without opening the database

### Requirement: Usage errors

Every other invocation (no arguments, unknown command or flag, missing or extra argument)
SHALL print the usage text to stderr, nothing to stdout, and exit 2, and SHALL do nothing
else: no file created, modified or locked, no database opened, no network connection, and an
outcome independent of whether the configuration is present or valid.

#### Scenario: Malformed invocations

- **WHEN** `kohaku`, `kohaku frobnicate`, `kohaku serve --port 9000`, `kohaku backup`, `kohaku restore --list extra` or `kohaku restore a.db b.db` runs, with a valid configuration or with no `KOHAKU_` variable
- **THEN** stderr holds the usage text, stdout is empty, the exit status is 2 (never a configuration failure), nothing listens on port 8080 or 9000 and `/data` is unchanged

### Requirement: Backup and restore through standard streams

`kohaku backup -` SHALL write to stdout exactly the backup `backup <file>` produces
(data-storage) and nothing else, exit 0 only after its last byte, leave stdout empty when it
fails before the first byte, and exit 1 when writing to stdout fails. `kohaku restore -`
SHALL read stdin to end of input, then behave exactly as `restore <file>` does for that
content (data-storage), every refusal included; an empty stdin or input that is not a
complete SQLite database SHALL be refused with exit 1 and the database left unchanged.

#### Scenario: Round trip through pipes

- **WHEN** `kohaku backup - > f` runs, the server is stopped and `kohaku restore - < f` runs
- **THEN** both exit 0 with all log lines on stderr; `f` starts with the 16-byte SQLite header (`SQLite format 3` and NUL) and passes an integrity check; every table holds its backup-time rows, apart from what data-storage and audit-log define a restore to add

#### Scenario: Broken or incomplete streams

- **WHEN** the reader of `kohaku backup -` closes the pipe after 4 KiB, or `kohaku restore -` receives an empty stdin or the first half of a backup
- **THEN** each exits 1 (the backup naming the write failure on stderr) and the database is unchanged

### Requirement: Listing backups

`kohaku restore --list` SHALL print the name, without directory, of every regular file in
`/data/backups/`, one per line ending in a line feed, in ascending byte order, and exit 0,
printing nothing for an empty or missing directory. It SHALL NOT create the directory, open
the database, modify any file or take the instance lock, so it works while `serve` runs.

#### Scenario: Files are listed by name while the server runs

- **WHEN** `kohaku restore --list` runs while `kohaku serve` holds the instance lock, with `/data/backups/` holding `pre-migrate-v1-1790000000.db` and `2026-09-23.db`, or missing
- **THEN** it exits 0 printing exactly `2026-09-23.db\npre-migrate-v1-1790000000.db\n`, or nothing without creating the directory, and the server keeps running and holding the lock

### Requirement: Server start

`kohaku serve` SHALL accept connections only after every startup check of configuration and
data-storage (validation, instance lock, instance-secret keycheck, migrations) has passed,
and on any failure SHALL exit 1 without having accepted one. It SHALL serve plain HTTP (TLS
ends at the reverse proxy) on TCP port 8080 of every local IPv4 address and, when the host
has IPv6, every local IPv6 address; the port SHALL NOT be configurable.

#### Scenario: Reachable on IPv4 and IPv6 loopback

- **WHEN** `kohaku serve` has started with a valid configuration and database
- **THEN** `GET http://127.0.0.1:8080/healthz` and, on a host with IPv6, `GET http://[::1]:8080/healthz` answer 200

#### Scenario: No request is served before startup has finished

- **WHEN** an attacker connects to port 8080 repeatedly while `kohaku serve` applies migrations, or while a start with a required setting missing fails
- **THEN** every connection is refused until startup has finished, so none meets a partly migrated database, and the failed start exits 1 having accepted none

### Requirement: Health endpoint

Kohaku SHALL answer `GET /healthz` with 200, `Content-Type: application/json` and exactly
`{"status":"ok"}` whenever the server accepts requests (so a 200 means startup completed),
whatever the `Host` (or none), query string and other headers; `HEAD` gets the same without
a body. The response SHALL NOT reveal the version, build, uptime, host name, configuration,
database state or any part of the request. Answering SHALL NOT use the database or any
concurrency permit or budget shared with other requests, nor log the request individually.
Other methods are answered as host-routing defines for the internal endpoints.

#### Scenario: Attacker input is neither reflected nor answered with internals

- **WHEN** an attacker sends `GET` or `HEAD /healthz?probe=<script>x</script>` with the main host, `Host: unknown.example` or no `Host`, plus an `X-Forwarded-For` entry and a `Cookie`, each holding a distinctive value
- **THEN** the status is 200 with `Content-Type: application/json`, the body is exactly `{"status":"ok"}` (none for `HEAD`), and no header or body contains those values or the binary's version

#### Scenario: Attacker floods the health endpoint

- **WHEN** an attacker sends `/healthz` requests continuously while every database connection and concurrency permit is held by other work
- **THEN** each is answered 200 without waiting for that work and the log gains no line for any of them

### Requirement: Healthcheck command

`kohaku healthcheck` SHALL send one `GET /healthz` directly to 127.0.0.1 port 8080 and exit 0
if and only if it receives status 200 within 5 s of starting. On a refused connection,
timeout, malformed response or any other status, a redirect included (never followed), it
SHALL exit 1, unhealthy to container health checks, with a one-line reason on stderr. It
SHALL ignore proxy environment variables, SHALL NOT read the configuration, secrets or
database, and SHALL write nothing to stdout.

#### Scenario: Healthy server

- **WHEN** `kohaku serve` runs and `kohaku healthcheck` runs in its network namespace with no `KOHAKU_` variable, no access to `/data`, and `HTTP_PROXY`, `http_proxy` and `ALL_PROXY` pointing at an unreachable address
- **THEN** the exit status is 0 and stdout and stderr are empty

#### Scenario: Unhealthy server

- **WHEN** nothing listens on 127.0.0.1 port 8080, or the listener never responds, or it answers 503, or 308 with a `Location` header
- **THEN** it exits 1 with empty stdout and a one-line stderr reason (naming the refused connection when nothing listens): at 5 s for the silent listener, after exactly one request for 503 and 308

### Requirement: Graceful shutdown

On SIGTERM, `kohaku serve` SHALL stop accepting connections, close idle ones, give requests
in flight at most 8 s and then close the connections of those unfinished, close every
database connection and exit 0, the instance lock released by exit; with nothing in flight
it SHALL exit without waiting out the 8 s. Background work SHALL NOT start new jobs after
SIGTERM or extend the 8 s.

#### Scenario: Idle server stops at once and cleanly

- **WHEN** SIGTERM arrives with no request in flight and one idle keep-alive connection open
- **THEN** that connection is closed and the process exits 0 within 8 s; with no other process holding the database, no `kohaku.db-wal` remains in `/data` and `kohaku restore <file>` can take the lock

#### Scenario: Request in flight completes, new connections are refused

- **WHEN** SIGTERM arrives while a client is sending a `GET /` it finishes 1 s later, and another client connects 1 s after SIGTERM
- **THEN** the `GET /` receives its normal response, the new connection is refused or closed unserved, and the process then exits 0

#### Scenario: Attacker holds a request open

- **WHEN** an attacker keeps a request in flight by sending one byte per second and SIGTERM arrives
- **THEN** its connection is closed within 8 s of SIGTERM and the process exits 0 within 10 s, the default container stop timeout

### Requirement: Log content privacy

Kohaku's logs, meaning everything `serve` logs and everything any command writes to stderr,
SHALL NOT contain at any level:

- the IP address or network prefix of anything that connects (peer, `X-Forwarded-For`);
- email addresses, tokens, cookie or `Authorization` values, secrets or keys;
- a request's path, query string, other header values or body content;
- any report, note or other user-submitted text.

A per-request log line SHALL identify the request only by its matched route pattern (with
placeholders, never their values) and response status, plus opaque internal ids where
needed. This SHALL also hold for error text from dependencies and remote servers, and no
setting or environment variable, `RUST_LOG` included, SHALL relax it. The only exception is
the full mail a `dev` build prints instead of sending it (mail-outbox); a build without `dev`
has none.

#### Scenario: Attacker-supplied data never reaches the log

- **WHEN** at the default level and at `RUST_LOG=trace`, an attacker sends accepted, rejected and unmatched requests carrying a distinctive email address, token-like strings in path, query, `Cookie` and `Authorization`, a distinctive `X-Forwarded-For` IP and distinctive body text
- **THEN** none of those values appears anywhere in the log, and the line for a route with a path parameter holds its pattern with the placeholder and the status

### Requirement: Rejected requests are logged as per-minute counters

`kohaku serve` SHALL NOT log rejected requests individually: it SHALL count them per reason
and every 60 s write one line per non-zero reason holding only reason and count, then reset.
Reasons SHALL come from a fixed built-in set, never from request data, including at least
unknown host (421), cross-site/origin rejection (403, Fetch-Metadata/Origin check), body over
its cap (413), multipart not accepted (415), request deadline exceeded (408), rate limit
exceeded (429) per rate-limit class and full concurrency bound (503) per bound.

#### Scenario: Attacker flood produces one line per reason

- **WHEN** within one 60 s interval an attacker sends 1 000 requests with 1 000 different unknown `Host` values and 1 000 requests answered 503 by a full concurrency bound
- **THEN** stdout holds exactly one unknown-host line and one line for that bound, each with count 1 000, and no per-request line (no error line for the 503s) or `Host` value

#### Scenario: Counts are per reason and restart each interval

- **WHEN** 3 requests are answered 429 in the `read` class and 2 answered 403 by the Fetch-Metadata/Origin check in one interval, 1 more `read` 429 in the next, and none in the third
- **THEN** the first interval gives a `read` line with 3 and a cross-site line with 2, the second a `read` line with 1, the third no line

### Requirement: Secrets are never arguments or output

No command SHALL accept a password, secret, key or token as a command-line argument; secrets
come only from the environment variables `KOHAKU_SECRET` and `KOHAKU_SMTP_PASSWORD`
(configuration). No command SHALL write either value, or any key or value derived from them,
to stdout, stderr or its log, and none SHALL print or log its environment or any variable's
value from it other than a validated non-secret setting.

#### Scenario: Secret-bearing flags are refused

- **WHEN** `kohaku serve --secret abc` or `kohaku serve --password abc` runs
- **THEN** each exits 2 with the usage text on stderr and starts no server

#### Scenario: Secret never echoed

- **WHEN** any command runs with secrets of known value and succeeds or fails, including with a malformed `KOHAKU_SECRET`, one that does not match the database, and an SMTP server rejecting the password
- **THEN** neither value nor its bytes in hex or base64 appear on stdout, stderr or in the log
