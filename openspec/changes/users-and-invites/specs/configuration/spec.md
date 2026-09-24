# Spec Delta

## MODIFIED Requirements

### Requirement: Startup validation and error reporting

`kohaku serve` SHALL validate every setting and decode the instance secret before it creates or
opens anything in the data directory or listens on any port. On invalid configuration it SHALL
report every problem of the run, each naming the variable and the broken rule, then exit
non-zero.

- Values SHALL be used exactly as given, never trimmed, case-folded or repaired; a value that is
  not valid UTF-8 is invalid. No error or log line SHALL contain a secret's value, valid or not.
- Kohaku SHALL refuse to start while `KOHAKU_BASE_URL`, `KOHAKU_SMTP_HOST`, `KOHAKU_SMTP_USERNAME`
  or `KOHAKU_SMTP_FROM` equals its shipped `.env.example` placeholder (the base URL as
  `compose.yaml` builds it from the `KOHAKU_DOMAIN` placeholder), naming each; the check SHALL
  cover every setting `.env.example` fills with a placeholder instead of a working value.
- `kohaku backup` and `kohaku admin unlock` SHALL require and validate only `KOHAKU_SECRET`;
  `kohaku project create` and every other `admin` command only `KOHAKU_SECRET` and
  `KOHAKU_BASE_URL`; `kohaku restore`
  and `kohaku healthcheck` SHALL require no setting from this spec. Any setting a command reads SHALL
  be validated by the `kohaku serve` rules.

#### Scenario: Every problem is reported in one run
- **WHEN** `kohaku serve` starts on an empty data directory with `KOHAKU_TRUSTED_PROXIES` unset, `KOHAKU_SMTP_FROM` empty and `KOHAKU_SMTP_PORT` set to ` 587`
- **THEN** it exits non-zero naming all three (the first two as missing, no default assumed), the data directory stays empty and nothing listens on port 8080

#### Scenario: Unchanged example configuration
- **WHEN** `kohaku serve` starts with both secrets set and otherwise an unchanged copy of the shipped `.env.example` and `compose.yaml`, or with only `KOHAKU_SMTP_FROM` still at its placeholder
- **THEN** it exits non-zero naming as unchanged example values all four settings, or only `KOHAKU_SMTP_FROM`

#### Scenario: Commands need only their own settings
- **WHEN** no setting from this spec is present and `kohaku healthcheck`, `kohaku restore <file>` and `kohaku backup -` run, the backup once more with a valid matching `KOHAKU_SECRET`
- **THEN** only the first backup fails for a missing setting, naming `KOHAKU_SECRET` and writing nothing; the second writes the backup
- **AND** `kohaku admin reset-password --email a@b.test` with only `KOHAKU_SECRET` set fails naming `KOHAKU_BASE_URL` and prints no link

### Requirement: Database keycheck

The database SHALL hold one keycheck, the value derived with label `kohaku/keycheck`. On a newly
created database without one, `kohaku serve` SHALL store the configured secret's keycheck before
accepting any request, and every other command except `kohaku restore` and `kohaku admin rekey` SHALL refuse the
database. Otherwise every command that opens the database except those two SHALL compare
the stored keycheck with the configured secret's before applying a migration, copying the
database, writing any row or listening on any port; on a mismatch it SHALL exit non-zero stating
that `KOHAKU_SECRET` does not match the database, with neither the secret nor either
keycheck in the error. `kohaku restore` and `kohaku admin rekey` SHALL NOT compare it: the
first replaces the database (the next command opening it compares), the second replaces the
keycheck (users-and-invites).

#### Scenario: Wrong secret refused
- **WHEN** `kohaku serve` starts on an empty data directory with secret A, restarts with A, then starts with secret B
- **THEN** both A starts serve requests; the B start exits non-zero with the mismatch error, schema version and rows unchanged, no file added to `/data/backups`, nothing listening on port 8080

#### Scenario: Backup refused with the wrong secret
- **WHEN** `kohaku backup <file>` runs on a database created with secret A while `KOHAKU_SECRET` holds secret B
- **THEN** it exits non-zero with the mismatch error and `<file>` is not created

#### Scenario: Restore skips the keycheck
- **WHEN** `kohaku restore <file>` restores a backup made under secret A while `KOHAKU_SECRET` holds B or is unset
- **THEN** it completes; the next `kohaku serve` exits with the mismatch error under B and starts under A
