# Spec Delta

## MODIFIED Requirements

### Requirement: Recovering from a lost instance secret

`kohaku admin rekey`, run with the server stopped, SHALL hold the instance lock, skip the
keycheck and, in one transaction with a new `KOHAKU_SECRET`: remove every account's TOTP
enrollment and recovery codes, end every session, clear every other stored value derived from
the old secret (the source keys of pending reports, report-submission; later capabilities
name theirs), store the new keycheck
and create a setup link for every account that had TOTP enrolled. It SHALL print one line per
such account, the account's address, a tab and its link, to stdout.

#### Scenario: Secret lost, backup restored
- **WHEN** an operator restores a backup made under a secret they no longer have, sets a new `KOHAKU_SECRET` and runs `kohaku admin rekey` while `kohaku serve` is stopped
- **THEN** it prints a setup line for each previously enrolled account, `kohaku serve` then starts under the new secret, and no old TOTP code or session works

#### Scenario: Rekey while the server runs
- **WHEN** `kohaku admin rekey` runs while `kohaku serve` holds the instance lock
- **THEN** it exits 1 saying another Kohaku process uses `/data` and changes nothing
