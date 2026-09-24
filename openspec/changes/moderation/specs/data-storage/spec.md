# Spec Delta

## MODIFIED Requirements

### Requirement: Retention job

`kohaku serve` SHALL run the retention job right after startup (after migrations) and then
every 60 minutes. Each run SHALL delete:

- outbox rows marked sent or given up;
- unsent outbox rows past their deadline (mail-outbox: the earlier of 24 hours after queuing
  and their expiry time), first given up as mail-outbox defines so a token-bearing row's
  token stops being accepted in the same transaction;
- audit log entries more than 365 days old;
- reports rejected as spam more than 30 days ago, with everything stored for them;
- sessions past their idle or absolute expiry (admin-auth);
- reset, setup and invitation tokens used or expired;
- device cookies (admin-auth) created more than 365 days ago;
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

#### Scenario: Expired sign-in state is purged

- **WHEN** the job runs on a session unused for 13 hours, one created 8 days ago, a fresh one, a used reset token, an expired one, a valid one, and device cookies created 366 and 10 days ago
- **THEN** only the fresh session, the valid token and the 10-day-old device cookie remain

#### Scenario: Only stale temporary files are removed

- **WHEN** `/data` and `/data/backups` each hold a `*.tmp` modified 25 hours ago and one 1 hour ago, and `/data/backups` also holds a backup and pre-migration copies, a subdirectory with a `*.tmp` and a symbolic link `old.tmp`, all 30 days old
- **THEN** only the two 25-hour-old files are deleted; the link's target and `/data/kohaku.db`, its WAL and SHM files and `/data/kohaku.lock` are unchanged too
