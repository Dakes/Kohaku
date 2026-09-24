# Spec Delta

## Purpose

Kohaku's single SQLite database: connection settings, atomic limit enforcement, secure
deletion, forward-only migrations, the instance lock, backup, restore and retention.
Mechanics follow design §3 "DB access" and "Migration rules" and §10 "Operations".

## ADDED Requirements

### Requirement: Database file and connection settings

Kohaku SHALL keep all instance data in `/data/kohaku.db` (plus `-wal` and `-shm`), with
every table `STRICT`. Every connection SHALL use `journal_mode=WAL`, `synchronous=NORMAL`,
foreign keys on (off only during a migration), `busy_timeout=5000` and
`journal_size_limit=67108864`, the writer also `secure_delete=ON`. A `kohaku serve` process
SHALL have at most 1 write connection, carrying every write (requests, migrations,
background work), and at most 4 read connections, which refuse writes. Work finding every
connection of its kind busy SHALL wait, counting toward an HTTP request's http-security
deadline, and SHALL NOT open another.

#### Scenario: Concurrent reads stay within the connection bound

- **WHEN** an attacker sends 100 concurrent read requests
- **THEN** `serve` never has more than 4 read and 1 write connection open, and each request is answered or ends at its deadline

### Requirement: Atomic limit enforcement

Every limit, counter or single-use value in the database SHALL be checked and updated by one
atomic operation on the write connection whose result (rows affected or returned) alone
decides the attempt; a read-connection check SHALL only reject early, never grant. Under any
concurrency, successes SHALL NOT exceed what the limit has left, a single-use value SHALL be
accepted at most once and no stored counter SHALL pass its limit.

#### Scenario: Parallel attempts

- **WHEN** an attacker sends 20 parallel redemptions of one single-use value, and 20 parallel attempts against a limit with 5 uses left
- **THEN** exactly 1 redemption succeeds and is recorded once, at most 5 attempts succeed, the counter never passes the limit, and an attempt whose read check saw a use left is rejected once the limit is used up

### Requirement: Secure deletion and WAL checkpoints

`kohaku serve` SHALL run a truncating WAL checkpoint at least every 60 minutes and after
every retention run, so deleted data is gone from `/data/kohaku.db` and its WAL once one
completes. A checkpoint blocked by a running read SHALL be retried at the next scheduled one
and SHALL NOT fail any request. Deletion SHALL NOT alter earlier backups or pre-migration
copies; the README SHALL state that these keep deleted data until deleted themselves.

#### Scenario: Deleted data cannot be recovered from the live files

- **WHEN** Kohaku deletes a row holding a unique marker, 60 minutes pass with no transaction open at the checkpoint, and an attacker copies `/data/kohaku.db` and `/data/kohaku.db-wal`
- **THEN** the WAL was 0 bytes right after the checkpoint and the marker occurs in neither file

### Requirement: Schema version and migrations at startup

The schema version SHALL be SQLite's `user_version`, the number of the newest applied
migration. Migrations SHALL be numbered 1, 2, 3, … without gaps and embedded in the binary;
each OpenSpec change SHALL add at most one (edited in place until merged), and a released one
SHALL NOT be edited, renumbered or removed. Before accepting HTTP requests, holding the
instance lock, `kohaku serve` SHALL create `/data/kohaku.db` if missing and apply each
migration above the schema version once, in ascending order; `kohaku backup` and
`kohaku restore` SHALL NOT migrate. Each migration and its version bump SHALL run in one
transaction with foreign keys off (on again for every later statement) and SHALL commit only
if SQLite's foreign-key check on the whole database finds nothing; a violation or failed
statement SHALL roll it back, keep the previous version and make `serve` exit non-zero naming
the migration, while migrations committed earlier in that start stay applied.

#### Scenario: New, older and current databases

- **WHEN** `serve` with newest migration n starts on no database, on one at version k < n, or on one at version n
- **THEN** it applies migrations 1…n, k+1…n or none, in ascending order, reaches version n and only then accepts requests

#### Scenario: Failed or interrupted migration

- **WHEN** migration k, applied at version k-1, leaves a dangling foreign key, or the process is killed while applying it
- **THEN** none of its changes remain and the version stays k-1 (earlier migrations of that start stay); a violation makes `serve` exit non-zero naming k, and after a kill the next start applies k from the beginning

#### Scenario: Table rebuilds keep child rows

- **WHEN** for each version k a new database is migrated to k, one row is inserted into every table and the remaining migrations are applied
- **THEN** no row was deleted or changed by `ON DELETE`/`ON UPDATE` actions, every table keeps its row count, the foreign-key check finds nothing and every connection then reports `foreign_keys` 1

### Requirement: Downgrade guard

When the schema version exceeds the binary's newest migration, `kohaku serve` and every
other command reading or changing application data, except `kohaku backup` and
`kohaku restore`, SHALL exit non-zero before writing anything (no database change, no
pre-migration copy, no HTTP request accepted), with an error stating both versions and that
the binary is older than the database.

#### Scenario: Older release started on a newer database

- **WHEN** `serve` whose newest migration is 3 starts on a version-4 database, and `kohaku backup` from that binary runs after it
- **THEN** `serve` exits non-zero naming 4 and 3, the database is unchanged, `/data/backups` gains no file and no request is answered; the backup is written at version 4

### Requirement: Pre-migration copy

When the schema version is at least 1 and below the newest migration, `kohaku serve` SHALL,
after the downgrade guard and before any migration, write a consistent copy to
`/data/backups/pre-migrate-v{old}-{unixtime}.db` (`{old}` the current version, `{unixtime}`
whole seconds since the Unix epoch), creating `/data/backups` with mode 0700 when missing.
The copy SHALL have mode 0600 from creation and get its final name only once complete and
flushed to stable storage. If it cannot be completed, `serve` SHALL remove it, apply no
migration and exit non-zero with an error. After a complete copy, Kohaku SHALL delete every
`pre-migrate-v*-*.db` except the 2 with the highest `{unixtime}`, and no other file.

#### Scenario: Copy before upgrading

- **WHEN** `serve` with newest migration 4 starts on a version-3 database while `/data/backups` holds `pre-migrate-v1-1790000000.db`, `pre-migrate-v2-1795000000.db` and `nightly.db`
- **THEN** before migration 4 runs, `pre-migrate-v3-{unixtime}.db` exists with mode 0600 as a version-3 database with the pre-start rows, and of the other files only `pre-migrate-v1-1790000000.db` is deleted

#### Scenario: A failed copy blocks the upgrade

- **WHEN** the copy cannot be completed, for example because the filesystem is full
- **THEN** no partial `pre-migrate-` file remains, no earlier copy is deleted, no migration is applied and `serve` exits non-zero

### Requirement: Instance lock

`kohaku serve` (before opening the database, until it exits) and `kohaku restore` (before
writing to `/data`, until it finishes) SHALL hold an exclusive lock on `/data/kohaku.lock`,
creating the file if missing. When the lock is held, they SHALL exit non-zero with an error
saying another Kohaku process is using `/data`, without opening the database or creating,
changing or deleting any file. The operating system SHALL release the lock whenever the
holder ends. `kohaku backup` and `kohaku restore --list` SHALL NOT need the lock.

#### Scenario: Concurrent commands and a crashed holder

- **WHEN** while `serve` holds the lock a second `serve`, a `restore`, a `backup` and a `restore --list` start, and then the holder is killed with SIGKILL
- **THEN** the second `serve` and the `restore` exit non-zero with that error and create or change no file; backup and listing complete while `serve` keeps answering; afterwards a new `serve` takes the lock without anyone deleting `/data/kohaku.lock`

### Requirement: Backup to a file

`kohaku backup <file>` SHALL write a consistent snapshot of the committed contents of
`/data/kohaku.db` to `<file>` as one self-contained SQLite file, the complete backup, never
holding the instance secret, while `serve` keeps answering requests. It SHALL write a new
`*.tmp` created with mode 0600 in `<file>`'s directory, flush it, publish it by an
operation that fails when any entry exists at `<file>` (a hard link, then removing the
temporary name) and flush the directory entry before exiting 0. It SHALL NOT replace or
modify an existing entry at `<file>` (file, directory or symbolic link, dangling or not,
including one appearing during the backup), exiting non-zero with an error naming the
target. On any failure it SHALL exit non-zero, leaving no new file at `<file>` and no `*.tmp`.

#### Scenario: Backup while the server runs

- **WHEN** `kohaku backup /data/backups/2026-09-23.db` runs while `serve` commits writes
- **THEN** until it is published the only new file is a mode-0600 `*.tmp` beside the target; it exits 0 with a mode-0600 file that needs no `-wal` or `-shm`, passes the integrity and foreign-key checks and holds every transaction committed before it started, each wholly or not at all
- **AND** an attacker who obtains the file finds neither the instance secret's bytes nor its base64 text in it

#### Scenario: Existing target or failure leaves nothing

- **WHEN** the target exists as a file, a directory or a symbolic link (dangling or not), or its directory is missing or its filesystem full
- **THEN** the command exits non-zero, any existing entry and link target are unchanged and no temporary file remains

### Requirement: Backup to standard output

`kohaku backup -` SHALL write the same snapshot to a new `*.tmp` created with mode 0600 in
`/data/backups` (created 0700 when missing), copy its bytes unchanged to standard output,
which carries nothing else, and delete the `*.tmp` whether or not the copy succeeded; a
failed write to standard output SHALL make it exit non-zero. `kohaku restore -` SHALL accept
the output.

#### Scenario: Streaming backup

- **WHEN** `docker compose exec -T kohaku kohaku backup - > file` runs beside `serve`, and in a second run the reader closes standard output early
- **THEN** the first exits 0 and the file is only a SQLite database that needs no `-wal` or `-shm`, passes the integrity check and holds the committed data; the second exits non-zero; neither leaves a temporary file

### Requirement: Restore

`kohaku restore <file>` and `kohaku restore -` (standard input) SHALL replace
`/data/kohaku.db` with a backup while holding the instance lock (`restore --list`: see
operations). They SHALL copy the whole source into a new `*.tmp` in `/data`, created with mode
0600, owned by the running user (uid 65532 in the published image), flush it, and refuse
a source that is empty, not SQLite, fails SQLite's integrity check or has schema version 0.
Only once the checked copy holds the restore entry audit-log requires and is flushed SHALL
they remove the old WAL and SHM files, rename the copy to `/data/kohaku.db` and flush
`/data`, then exit 0. On refusal or failure they SHALL exit non-zero, delete the `*.tmp` and
leave the database and its WAL and SHM files unchanged. Restore SHALL NOT migrate: the next
`serve` migrates the database (after a pre-migration copy) or its downgrade guard refuses it.

#### Scenario: Restore from standard input

- **WHEN** with `serve` stopped and the old WAL non-empty, `docker compose run --rm -T kohaku kohaku restore - < file` runs on output of `kohaku backup -`
- **THEN** it exits 0 and `/data/kohaku.db` has mode 0600 and holds the backup's data plus only the restore entry, none of the old rows the backup lacks; no temporary file remains

#### Scenario: Bad source or interrupted restore

- **WHEN** an attacker or faulty transfer supplies an empty, truncated or random source or one with a valid header and corrupted pages, or a restore is killed while copying or checking
- **THEN** the database and its WAL and SHM files are unchanged; a refusal exits non-zero (for a damaged source saying it is not a valid backup) and leaves no `*.tmp`; a kill leaves at most one, which retention removes after 24 hours

### Requirement: Retention job

`kohaku serve` SHALL run the retention job right after startup (after migrations) and then
every 60 minutes. Each run SHALL delete:

- outbox rows marked sent or given up;
- unsent outbox rows past their deadline (mail-outbox: the earlier of 24 hours after queuing
  and their expiry time), first given up as mail-outbox defines so a token-bearing row's
  token stops being accepted in the same transaction;
- audit log entries more than 365 days old;
- regular `*.tmp` files directly in `/data` or `/data/backups` modified over 24 hours ago.

It SHALL delete no other file, follow no symbolic link and enter no subdirectory; every
temporary file Kohaku creates there SHALL be named `*.tmp`. Each run SHALL end with a
truncating WAL checkpoint. A failed run SHALL be logged without row content, SHALL NOT stop
`serve` and SHALL be retried at the next scheduled run.

#### Scenario: Runs at startup and survives failure

- **WHEN** `serve` starts on a database holding a 366-day-old audit entry with no other transaction open, and a later run fails because the database stays locked past the busy timeout
- **THEN** the entry is deleted without waiting 60 minutes and the WAL is 0 bytes after that run; the failure is logged without row content, `serve` keeps answering and the job runs again 60 minutes later

#### Scenario: Finished and expired outbox rows are purged

- **WHEN** the job runs on a sent row, a given-up row, an unsent row past its expiry time, an unsent row queued 25 hours ago, and one queued 23 hours ago with a future expiry time
- **THEN** the first four are deleted and the last is kept

#### Scenario: Only stale temporary files are removed

- **WHEN** `/data` and `/data/backups` each hold a `*.tmp` modified 25 hours ago and one 1 hour ago, and `/data/backups` also holds a backup and pre-migration copies, a subdirectory with a `*.tmp` and a symbolic link `old.tmp`, all 30 days old
- **THEN** only the two 25-hour-old files are deleted; the link's target and `/data/kohaku.db`, its WAL and SHM files and `/data/kohaku.lock` are unchanged too
