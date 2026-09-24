# Spec Delta

## Purpose

Append-only, ids-only trail of every state-changing admin action and CLI command, kept
for 365 days (design baseline §4 `audit_log`). Viewing it is added by a later change.

## ADDED Requirements

### Requirement: Audited actions

Every state-changing admin action (by a signed-in admin or maintainer) and CLI command
SHALL record audit entries whose action identifier, target type and ids the change adding
it defines; in this change that is only `kohaku restore`. Nothing else SHALL, notably:

- an action that ends without changing stored state (refused, invalid or failed);
- routes needing neither a session nor an invite, reset or setup token (public pages,
  report forms, public JSON API, one-click unsubscribe, landing page, internal endpoints);
  in this change, any HTTP request on any host;
- `kohaku healthcheck`, `backup <file>`, `backup -`, `restore --list`, `serve` startup and
  the server's own work (startup migrations, retention, outbox delivery, DB maintenance).

#### Scenario: Anonymous request flood
- **WHEN** an attacker sends GET and POST requests with arbitrary paths, headers and bodies to every main-host route, `/admin/…`, unknown hosts, `/healthz` and `/.well-known/kohaku/tls-ask`
- **THEN** the number of audit entries is unchanged

#### Scenario: Read-only commands and background work
- **WHEN** an operator runs each read-only command above, `kohaku serve` starts and migrates an existing database, and the outbox worker sends a mail or gives up on one
- **THEN** the number of entries in the live database is unchanged

### Requirement: Entry fields

Each entry SHALL have exactly these fields; the database itself SHALL refuse an entry that
breaks any rule here, failing the recording action without changing stored state:

- actor label `cli`, `admin` or `maintainer`, and actor id, an integer absent if and only
  if the label is `cli`: a CLI command records `cli`, an admin action the acting user's id
  and role at that time; neither changes afterwards, whatever happens to the user;
- action, and target type (`instance` for the whole instance): one or more `.`-joined
  segments, each a lowercase ASCII letter then zero or more lowercase ASCII letters, digits or `_`;
- target id: integer, absent if and only if the target type is `instance`;
- time: UTC time the change was committed, one-second resolution.

#### Scenario: Entry read back
- **WHEN** an entry is recorded for user 7 as `maintainer`, action `sample.update`, target type `sample`, target id 42
- **THEN** it reads back with exactly those values plus the UTC commit time, and no other field

#### Scenario: Malformed, inconsistent or attacker-supplied values refused
- **WHEN** the action or target type is empty or has `@`, whitespace, an uppercase letter or a segment starting with a digit or `_`; the label is another value; actor id presence contradicts the label; target id presence contradicts the target type; an id is not an integer; or a code path passes attacker text such as `victim@example.com`, `203.0.113.7` or `Crash on start` as action, target type or target id
- **THEN** the database refuses the entry and the action fails without changing stored state

### Requirement: Ids only

Entries SHALL identify actors and targets by id only and record that a change happened,
never the values before or after it. They SHALL NOT contain email addresses, IP addresses,
network prefixes or values derived from them, report titles, bodies, notes or close
reasons, project names or slugs, host names, tokens, passwords, codes, or any part of a
request (path, query string, header value, body).

#### Scenario: Restore entry carries no file name or content
- **WHEN** an operator restores `victim@example.com-203.0.113.7.db`, whose rows contain the attacker-supplied text `Crash on start`
- **THEN** no entry the restore records contains those strings or any part of the file's path

### Requirement: Recorded together with the change

An entry SHALL be committed in the same database transaction as the change it records: a
committed change always has its entries, a rolled-back change leaves none, and if an entry
cannot be recorded the action SHALL fail and its change be rolled back.

#### Scenario: Entry and change are atomic
- **WHEN** an audited action commits, fails after recording its entry but before committing, or fails to record its entry (e.g. the database refuses it)
- **THEN** no reader ever sees the change without its entries; on failure the action reports it and neither change nor entry is stored

### Requirement: Entries outlive actors and targets

Entries SHALL hold no reference the database checks or cascades: an entry naming an actor
or target id that does not or no longer exists is stored and kept, and deleting, disabling
or changing any record never removes or alters an entry. An id that can appear in an entry
SHALL never be given to a different record of the same kind, even after deletion; in this
change no entry names a record, so the rule binds every kind later changes' entries name.

#### Scenario: Entry naming a missing record is kept
- **WHEN** an entry names actor and target id 999 that never existed, or 7 and 42 whose records (test-suite tables here) are then deleted
- **THEN** it is stored and reads back unchanged

#### Scenario: New record after deleting the newest one
- **WHEN** the highest-id record of a kind that entries name (a test-suite table here) is deleted and a new one is created
- **THEN** the new id was never used by that kind and no existing entry names the new record

### Requirement: Append-only

A committed entry SHALL never be modified and SHALL leave the live database only with the
whole database on `kohaku restore`, or through retention runs (see data-storage), which
delete exactly the entries whose time is more than 365 days before the run and record no
entry about it. No route on any host and no CLI command SHALL modify or delete individual
entries; the database itself SHALL refuse any field change and any deletion of an entry
≤ 365 days old.

#### Scenario: Retention purges only old entries
- **WHEN** the retention job runs with one entry 366 days old and one 364 days old
- **THEN** only the 366-day-old entry is deleted and no new entry is created

#### Scenario: Attacker tries to rewrite or erase a recent entry
- **WHEN** every route and CLI command other than `kohaku restore` is exercised, and an attacker who can make the application run statements on the audit log (e.g. through a bug) changes any field of an entry, such as its actor, or deletes one 364 days old
- **THEN** no entry is modified or deleted; the database refuses the attacker's statements

### Requirement: Restore is audited

A successful `kohaku restore <file>` or `kohaku restore -` SHALL leave the live database
holding exactly the restored file's entries, unchanged (none carried over from the
replaced database), plus one new entry committed with the restored data: label `cli`, no
actor id, action `instance.restore`, target type `instance`, no target id, restore time.
The entry SHALL be recorded in the restored database before that replaces the live one; a
restore that fails for any reason, this recording included, SHALL keep the live database
in place and record no entry anywhere.

#### Scenario: Restore leaves a trace
- **WHEN** an operator restores a backup whose audit log holds 12 entries
- **THEN** the live database holds exactly those 12, unchanged, plus the one `instance.restore` entry

#### Scenario: Failed restore records nothing
- **WHEN** the instance lock is held, an attacker-supplied file fails the integrity check, or the file passes it but cannot take the entry (e.g. an SQLite database without an audit log)
- **THEN** the restore exits non-zero, the live database is unchanged and no entry is recorded in it or in any other file
