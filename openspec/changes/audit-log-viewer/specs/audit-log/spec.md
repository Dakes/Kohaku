# Spec Delta

## ADDED Requirements

### Requirement: Viewing the audit log

The admin SHALL read the audit log at `/admin/audit` (404 for maintainers), newest first, 50
entries per page with a keyset cursor on the entry id, filterable by action and by target
type and id. Each entry SHALL show its time (UTC), actor label, action and target as stored,
plus, looked up when the page renders and never stored, the current email of an account or
the current slug of a project it names, or "deleted" when that record no longer exists.
Viewing SHALL change nothing and record no entry. A malformed cursor or filter SHALL get 400.

#### Scenario: Entries survive deleted records
- **WHEN** the admin views entries naming a project that was since deleted and a maintainer who was since disabled
- **THEN** both entries are listed with their ids, the project shown as "deleted" and the maintainer with their current address

#### Scenario: Maintainer or attacker probes the log
- **WHEN** a maintainer requests `/admin/audit`, and the admin's browser requests `?cursor=%ff&action=<script>`
- **THEN** the maintainer gets 404, the admin gets 400, and neither request adds an entry
