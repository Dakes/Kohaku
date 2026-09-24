# Spec Delta

## MODIFIED Requirements

### Requirement: Grants and project access

A maintainer SHALL work only on projects granted to them; the admin works on every project.
Every admin route acting on a project SHALL be under `/admin/p/{slug}/`, SHALL bind the
project from the slug and a grant check in one lookup, and SHALL answer 404, identical to a
nonexistent project, when the project does not exist or the user has no grant. Routes the
projects capability reserves for the admin SHALL stay admin-only. `/admin` SHALL list the
projects the user may work on. The admin SHALL change a maintainer's grants on the user's page;
revoking a grant deletes the maintainer's unsent mail about that project in the same
transaction (notifications).

#### Scenario: Maintainer probes an ungranted project
- **WHEN** a maintainer granted only project A requests `/admin/p/b/…` for existing project B and nonexistent project C
- **THEN** both get the same 404 and nothing reveals that B exists

#### Scenario: Grant revoked mid-session
- **WHEN** the admin removes project A from a maintainer's grants while the maintainer is signed in
- **THEN** the maintainer's next request under `/admin/p/a/` gets 404
