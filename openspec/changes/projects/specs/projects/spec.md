# Spec Delta

## Purpose

Projects: what the admin creates, configures and deletes, the per-project switches later
capabilities read, optional custom domains with automatic certificates, and the redirect
from main-host project paths to a project's own domain (design §4 `projects`, §6 Domains
and host routing).

## ADDED Requirements

### Requirement: Project fields

A project SHALL have:

- `slug`: `[a-z0-9][a-z0-9-]{0,39}`, unique, set at creation and never changed;
- `name`: 1–80 Unicode scalar values after removing surrounding whitespace, single line;
- `public_host`: optional, see "Custom domains";
- switches, each off unless the admin turns it on: `screenshots_enabled`,
  `features_enabled` (reporters may file feature requests as well as bugs),
  `require_email` (reporters verify an email address), and `plus_one_enabled`, which is on
  for a new project;
- `privacy_notice`: optional plain text of at most 1024 bytes of UTF-8 after line endings are
  normalized to LF;
- `security_contact`: optional, at most 254 Unicode scalar values, single line.

"Single line" SHALL mean no character of Unicode category Cc, no U+2028 or U+2029, and none of
the invisible or bidirectional format characters U+00AD, U+061C, U+180E, U+200B–U+200F,
U+202A–U+202E, U+2060–U+2064, U+2066–U+206F, U+FEFF, U+FFF9–U+FFFB and U+E0000–U+E007F.
`privacy_notice` SHALL follow the same rule except that LF is allowed. A violation SHALL
be answered 422 with the form and a message naming the field, changing nothing. Project ids
SHALL never be reused, even after deletion.

#### Scenario: Attacker-controlled text cannot hide in a project name
- **WHEN** the admin's browser submits a name containing U+202E, a zero-width space, a tab or a line feed, or 81 characters, or a slug `Demo`, `-demo` or 41 characters long
- **THEN** each gets 422 naming the field and no project is created or changed

#### Scenario: Defaults of a new project
- **WHEN** the admin creates project `demo` named `Demo app` with no other input
- **THEN** it has no custom domain, screenshots, feature requests and email verification are off, and +1 is on

### Requirement: Project administration

Only the admin SHALL list, create, change and delete projects, on the main host under
`/admin/projects` and `/admin/p/{slug}/settings`; for a signed-in maintainer every such path
SHALL answer 404, the same as a nonexistent path. `/admin` SHALL list every project for
the admin. Creating, changing and deleting SHALL each record one audit entry
(`project.create`, `project.update`, `project.delete`, target type `project`, the project's
id) in the same transaction. Deleting SHALL require typing the project's slug into the
confirmation form and SHALL remove the project and everything stored for it; a wrong or
missing slug SHALL change nothing.

#### Scenario: Maintainer probes the project settings
- **WHEN** a signed-in maintainer requests `/admin/projects`, `/admin/p/demo/settings` and posts to `/admin/p/demo/delete` with a valid CSRF token
- **THEN** the GETs get 404 with the body of a nonexistent `/admin` path, the POST gets 404, and nothing changes

#### Scenario: Deletion needs the slug
- **WHEN** the admin posts deletion of `demo` with the confirmation `dem`, then with `demo`
- **THEN** the first changes nothing; the second removes the project, answers `303` to `/admin/projects` and records `project.delete`

### Requirement: Custom domains

A `public_host` SHALL be a DNS name under the configuration capability's host rules with at
least one dot, not the main host, and unique across projects; the admin's input SHALL have
surrounding whitespace removed and ASCII letters lowercased before these checks. Kohaku
SHALL add it to the host map (host-routing) for that project as soon as the change that sets
it commits, and remove the old one, so the old host gets 421 and TLS asks for it 404 from
then on. Changes made by another process (the CLI) SHALL reach a running server's host map
within 2 seconds, without a restart.

#### Scenario: Attacker claims someone else's domain
- **WHEN** DNS for `evil.example` points at the server but no project has it, and later the admin moves project A from `bugs.example.net` to `bugs.example.org`
- **THEN** tls-ask answers 404 for `evil.example`; after the move it answers 404 for `bugs.example.net` and 200 for `bugs.example.org`, and requests with `Host: bugs.example.net` get 421

#### Scenario: Invalid or taken hosts
- **WHEN** the admin enters `192.0.2.1`, `localhost`, the main host, `bugs.example.net.` or the host of another project
- **THEN** each gets 422 naming the field and nothing changes

#### Scenario: CLI change reaches the running server
- **WHEN** `kohaku project create demo --name Demo --host bugs.example.net` runs while `kohaku serve` runs
- **THEN** within 2 seconds `bugs.example.net` is served by demo's project router and tls-ask answers 200 for it

### Requirement: Canonical redirect to the custom domain

For a project with a `public_host`, a `GET` or `HEAD` on the main host for `/p/{slug}` or any
path below it except `/p/{slug}/api/` and below SHALL be answered `308` with `Location`
`https://{public_host}` plus the path after `/p/{slug}` (or `/`) and the original query
string, and `Cache-Control: max-age=3600`, the only other value this response sets. Other
methods and the API paths SHALL stay on the main host. The location SHALL come only from the
stored host, never from request headers.

#### Scenario: Old links follow the domain
- **WHEN** project demo has `public_host` `bugs.example.net` and a client requests `GET /p/demo/r/12?x=1` and `HEAD /p/demo` on the main host with `X-Forwarded-Host: evil.example`
- **THEN** the responses are `308` to `https://bugs.example.net/r/12?x=1` and `https://bugs.example.net/`, each with `Cache-Control: max-age=3600`

#### Scenario: API and writes are never redirected
- **WHEN** a client sends `GET /p/demo/api/v1/reports` and `POST /p/demo/new` on the main host for that project
- **THEN** neither response is a redirect

### Requirement: Project creation from the command line

`kohaku project create <slug> --name <name> [--host <host>]` SHALL create a project with the
same rules and defaults as the admin form, record `project.create` with actor `cli`, print
nothing to stdout and exit 0; an invalid value or a taken slug or host SHALL exit 1 naming
the field on stderr and create nothing.

#### Scenario: Scripted setup
- **WHEN** `kohaku project create demo --name "Demo app"` runs twice
- **THEN** the first exits 0 and creates the project; the second exits 1 saying the slug is taken
