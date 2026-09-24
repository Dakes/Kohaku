# Tasks

## 1. Schema and validation

- [x] 1.1 Add `migrations/0003_projects.sql` (D1); verify the §12 migration test with a
  sample project and that ids are never reused after deletion.
- [x] 1.2 Add the field validators, `single_line` included (D6; projects: Project fields);
  verify "Attacker-controlled text cannot hide in a project name" as unit tests and each
  listed character.

## 2. Host map and routing

- [x] 2.1 Build the host map and slug table from `projects` and swap them (D4); verify
  host-routing "In-memory host map" and "TLS ask answers only for configured project
  hosts" with real project rows.
- [x] 2.2 Add the watcher connection (D4); verify "CLI change reaches the running server"
  within 2 s and that a failing rebuild keeps the old map.
- [x] 2.3 Add the canonical redirect (D5; projects: Canonical redirect to the custom
  domain); verify its two scenarios and matrix rows for its own Cache-Control.
- [x] 2.4 Verify host-routing "Project-host routes" and "Main-host routes" with a project
  that has a host and one that has none.

## 3. Administration

- [x] 3.1 Add the `Admin` access level (D2); verify "Maintainer probes the project settings"
  and a matrix row per admin route.
- [x] 3.2 Add the project list, create form and settings page with audit entries (D3;
  projects: Project administration, Custom domains); verify "Defaults of a new project",
  "Invalid or taken hosts", "Attacker claims someone else's domain" and `project.create` /
  `project.update`.
- [x] 3.3 Add deletion with typed-slug confirmation; verify "Deletion needs the slug" and
  that the host leaves the map at once.
- [x] 3.4 List projects on `/admin` for the admin.

## 4. CLI, smoke test and documentation

- [x] 4.1 Add `kohaku project create` (D7; projects: Project creation from the command line;
  operations; configuration); verify "Scripted setup", malformed invocations exit 2 and the
  `cli` audit entry.
- [x] 4.2 Extend `ci/smoke.sh`: `project create --host`, wait for the host map, the project
  host gets a certificate from Caddy's local CA and serves a static asset, `/p/{slug}`
  answers 308.
- [x] 4.3 README: creating projects, custom domains (DNS first, then the setting), deleting.
- [x] 4.4 Final check: the before-finishing commands and the smoke test.
