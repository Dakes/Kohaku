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
  `kohaku admin reset-password` and `kohaku project create` only `KOHAKU_SECRET` and
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
