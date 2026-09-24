# Spec Delta

## MODIFIED Requirements

### Requirement: Command-line commands

The `kohaku` binary SHALL accept exactly `serve`, `healthcheck`, `backup <file>`, `backup -`,
`restore <file>`, `restore -`, `restore --list`, `admin unlock --email <address>`,
`admin reset-password --email <address>`, `admin create --email <address>`,
`admin reset-2fa --email <address>`, `admin rekey`, `project create <slug> --name <name>` with an
optional `--host <host>` after the name, `--version`, `--help` and `<command> --help`
(`admin <subcommand> --help` and `project create --help` for the two-word commands), names and flags matched exactly, case included.

- `--version` SHALL print `kohaku <version>\n` to stdout, `<version>` being the built version,
  equal to the release tag without `v` (e.g. `1.0.0`); `--help` and `<command> --help` SHALL
  print the usage text, naming every invocation, to stdout and run nothing. Both exit 0 with
  no `KOHAKU_` variable, `/data` or network, and create or modify no file.
- Every command SHALL exit 0 on success, 2 on a usage error and 1 on any other failure, whose
  cause it first reports on its log stream.
- Stdout SHALL carry only data (the `backup -` backup, the `restore --list` listing, the
  links printed by `admin reset-password`, `admin create`, `admin reset-2fa` and
  `admin rekey`, the `--version` and `--help` text), so `admin unlock`,
`project create`, `healthcheck`, `backup <file>`, `restore <file>` and
  `restore -` write nothing there. Every command but `serve` SHALL log (warnings and errors
  included) to stderr only; `serve`, which has no data output, SHALL log to stdout.

#### Scenario: Version and help need no configuration

- **WHEN** `kohaku --version`, `kohaku --help` or `kohaku backup --help` runs with no `KOHAKU_` variable and no `/data`
- **THEN** each exits 0 and creates no file; `--version` prints exactly `kohaku <version>\n` with empty stderr; the help names `serve`, `healthcheck`, `backup`, `restore`, `restore --list`, every `admin` subcommand and `project create`

#### Scenario: A refusal is a failure, not a usage error

- **WHEN** `kohaku backup <file>` names an existing file, or `kohaku restore <file>` runs while `kohaku serve` holds the instance lock
- **THEN** the exit status is 1, stderr names the cause and stdout is empty

#### Scenario: Malformed admin invocations

- **WHEN** `kohaku admin`, `kohaku admin unlock`, `kohaku admin unlock --email`, `kohaku admin unlock --email a@b.test extra`, `kohaku admin unlock a@b.test` or `kohaku admin frobnicate` runs
- **THEN** each prints the usage text to stderr and exits 2 without opening the database
