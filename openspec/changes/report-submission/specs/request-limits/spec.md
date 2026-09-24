# Spec Delta

## ADDED Requirements

### Requirement: Submit rate-limit class

The `submit` class SHALL use the per-prefix buckets with budgets fixed in the binary: 10 per
hour per IPv4 /32 or IPv6 /64 and 80 per hour per IPv6 /48, covering `POST /new` and
`POST /api/v1/reports` on both hosts of every project, shared across projects. Challenge
requests belong to `read`.

#### Scenario: Attacker submits across projects
- **WHEN** an attacker at `203.0.113.5` posts 6 reports to project A and 5 to project B within one hour, each with a valid proof of work
- **THEN** 10 are processed and the 11th gets 429 without its body being read
