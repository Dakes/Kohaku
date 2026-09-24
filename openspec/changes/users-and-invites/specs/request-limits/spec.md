# Spec Delta

## MODIFIED Requirements

### Requirement: Login and reset rate-limit classes

Two more classes SHALL use the per-prefix buckets of "Rate-limit keys by network prefix",
with budgets fixed in the binary:

- `login`: 10 per 15 minutes per IPv4 /32 or IPv6 /64, 80 per 15 minutes per IPv6 /48. It
  SHALL cover every request that checks a password, a TOTP code or recovery code, or a
  single-use account token: `POST /admin/login`, `POST /admin/reset/{token}`,
  `POST /admin/setup/{token}`, `POST /admin/invite/{token}`, and every
  re-authenticated account change (admin-auth).
- `reset`: 5 per hour per IPv4 /32 or IPv6 /64, 40 per hour per IPv6 /48, covering
  `POST /admin/reset`.

A request SHALL draw only from its own class; `read` never covers these POSTs.

#### Scenario: Attacker guesses passwords from one network
- **WHEN** an attacker at `203.0.113.5` sends 11 login POSTs within one minute for 11 different accounts
- **THEN** 10 are processed and the 11th gets 429 without its body being read or a password hashed

#### Scenario: Attacker spreads over an IPv6 /48
- **WHEN** an attacker sends 10 login POSTs from each of 9 different /64s inside `2001:db8:5::/48` within 15 minutes
- **THEN** 80 are processed and the rest get 429
