# Spec Delta

## ADDED Requirements

### Requirement: OTP rate-limit class

The `otp` class SHALL use the per-prefix buckets with budgets fixed in the binary: 20 per
hour per IPv4 /32 or IPv6 /64 and 160 per hour per IPv6 /48, covering code sending and
checking on the pages and the API of every project.

#### Scenario: Attacker cycles codes
- **WHEN** an attacker at `203.0.113.5` sends 21 code requests or checks within one hour
- **THEN** the 21st gets 429 without its body being read
