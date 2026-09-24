# Spec Delta

## ADDED Requirements

### Requirement: +1 rate-limit class

The `plus_one` class SHALL use the per-prefix buckets with budgets fixed in the binary: 30 per
hour per IPv4 /32 or IPv6 /64 and 240 per hour per IPv6 /48, covering "+1" posts on the
pages and the API of every project.

#### Scenario: Attacker sprays votes across reports
- **WHEN** an attacker at `203.0.113.5` posts "+1" for 31 different reports within one hour
- **THEN** the 31st gets 429 without its body being read
