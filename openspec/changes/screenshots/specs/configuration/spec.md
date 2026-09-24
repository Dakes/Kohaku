# Spec Delta

## MODIFIED Requirements

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
- `KOHAKU_SCREENSHOT_QUOTA_PROJECT_MIB` and `KOHAKU_SCREENSHOT_QUOTA_INSTANCE_MIB`, optional:
  screenshot storage per project and per instance in MiB (screenshots); 256 and 1024 when
  unset, else a decimal 1–1048576 under the same rules as `KOHAKU_PUBLIC_MAIL_PER_HOUR`.
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

#### Scenario: Screenshot quota values
- **WHEN** both quotas are unset, or set to `512` and `2048`, or one is `0`, `-5`, `0256` or `1048577`
- **THEN** the quotas are 256/1024 or 512/2048 MiB; each other value makes `kohaku serve` exit non-zero naming the variable

#### Scenario: Development build takes no SMTP server
- **WHEN** a `dev` build's `kohaku serve` starts with `KOHAKU_SMTP_FROM` and no other SMTP setting, and again with `KOHAKU_SMTP_HOST` also set to the empty string
- **THEN** the first starts; the second exits non-zero naming `KOHAKU_SMTP_HOST`
