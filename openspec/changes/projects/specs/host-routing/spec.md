# Spec Delta

## MODIFIED Requirements

### Requirement: In-memory host map
Kohaku SHALL choose the router from an in-memory host map, without a database query:
- the main host (`KOHAKU_BASE_URL`'s host, lowercased, without port) maps to the main router
  whatever port `Host` names; names that contain it, subdomains included, are not the main host;
- each project host (configured for exactly one project) maps to the project router bound to
  that project, as the projects capability keeps it (changes apply when they commit, or within
  2 seconds when made by another process).

#### Scenario: Attacker uses lookalike or random hosts
- **WHEN** an attacker sends `Host` `evil.kohaku.example.org`, `kohaku.example.org.evil.example`, `bugs.example.net` (no project has it), then 10 000 random values
- **THEN** every response is 421 and no request performs a database query

### Requirement: TLS ask answers only for configured project hosts
`GET /.well-known/kohaku/tls-ask` SHALL answer 200 only when the query string has exactly one
`domain` parameter whose decoded value, normalized as `Host` is, is a project host in the host
map; otherwise 404, including for the main host, an unknown name, and a missing, empty or
repeated `domain`. It SHALL decide from the in-memory map alone,
without a database query; each status has one fixed body for every name, with no project data
(id, slug, name); requests SHALL NOT be logged individually and the queried name SHALL NOT appear
in logs.

#### Scenario: Configured project host in mixed case with a port
- **WHEN** the host map contains project host `bugs.example.net` and the ask is `domain=Bugs.Example.NET:443`
- **THEN** the response is 200

#### Scenario: Main host, unknown and malformed queries
- **WHEN** with that project host, the ask names the main host or `evil.example`, has no query, or is `?domain=` or `?domain=bugs.example.net&domain=evil.example`
- **THEN** each response is 404

#### Scenario: Attacker enumerates names
- **WHEN** an attacker sends 10 000 TLS asks, each for a different name no project has
- **THEN** every response is 404 with the same body, none queries the database, and the logs hold no per-request line and no queried name

### Requirement: Project-host routes
A project-host request SHALL be served by the project router bound to that host's project, which
has no path parameter selecting a project and restricts every data lookup to that project. `/p`,
`/p/*`, `/admin`, `/admin/*` and the login, invite, reset, setup and unsubscribe pages SHALL get
404 there. It SHALL serve the assets under `/static/` (plus `dev`-build routes) and the routes
later capabilities add to it; every other path gets 404.

#### Scenario: Attacker addresses another project or the admin area through a project host
- **WHEN** `bugs.example.net` maps to project A and an attacker requests `/p/b`, `/p/b/api/v1/reports`, `/admin` and `/admin/login` there
- **THEN** each response is 404 and none contains data of project B

### Requirement: Main-host routes
The main router SHALL serve only the landing page at `/`, the assets under `/static/`, the
`/admin` routes of the admin-auth and projects capabilities (admin-auth also defines how
`/admin` paths answer without a session), the projects capability's redirects under
`/p/{slug}`, and the routes later capabilities add; every other path gets 404. Only the main
host SHALL serve the admin area and the login, invite, reset, setup and unsubscribe pages.
Routes of a `dev` build (distribution capability) SHALL be absent without the `dev` feature,
on this router and on project routers. The landing page SHALL be 200 HTML, identical
whatever the query string or headers, with no project, report, user or configuration data (no
project names, slugs, hosts or counts, no configured values) and no cookie. It SHALL change no
state: a `POST /` that passes every `http-security` and `request-limits` check (Fetch-Metadata
and Origin, multipart, body cap) gets 405.

#### Scenario: Attacker probes for the admin area
- **WHEN** an attacker without a session requests `GET /admin`, `/admin/users`, `/p/demo` and `/p/demo/api/v1/reports` on the main host
- **THEN** both `/admin` paths get the same `303` to `/admin/login`, and, with no project `demo`, both `/p/` paths get 404 with the status and body of `GET /does-not-exist`

#### Scenario: Attacker tries to reflect input into the landing page
- **WHEN** an attacker requests `GET /?q=<script>alert(1)</script>` on the main host with `X-Forwarded-Host: evil.example`
- **THEN** it gets 200, `Content-Type: text/html; charset=utf-8`, no `Set-Cookie`, no project listed, and the body of a plain `GET /`

#### Scenario: POST to the landing page
- **WHEN** a client sends `POST /` on the main host with `Origin: https://kohaku.example.org` matching `KOHAKU_BASE_URL`
- **THEN** the response is 405
