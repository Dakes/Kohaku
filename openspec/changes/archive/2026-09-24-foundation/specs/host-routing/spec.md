# Spec Delta

## Purpose

Choose the router for a request from its `Host` header alone, reject hosts Kohaku does not
serve, answer the two internal endpoints for any `Host`, and build absolute URLs and cookies
only from configured hosts (design baseline §6, "Domains and host routing").

## ADDED Requirements

### Requirement: Host normalization
Kohaku SHALL derive the request host from the `Host` header alone and route a request only when
its normalized host equals a host-map entry exactly:
- normalization removes one trailing `:` plus one or more ASCII digits and lowercases ASCII
  letters, nothing else (no trailing-dot removal, percent-decoding or IDN conversion);
- a missing, empty or repeated `Host` header counts as an unknown host;
- no other header (`X-Forwarded-Host`, `X-Forwarded-Proto`, `Forwarded`, `X-Real-IP`, …) SHALL
  influence the chosen router, whether the host is known, or the project a router is bound to.

#### Scenario: Mixed case and a port are normalized
- **WHEN** `KOHAKU_BASE_URL` is `https://kohaku.example.org:8443` and `GET /` arrives with `Host: Kohaku.Example.ORG:8080`
- **THEN** the main router serves the landing page with status 200

#### Scenario: Attacker sends a malformed, repeated or missing Host
- **WHEN** an attacker sends `Host` `kohaku.example.org.`, `kohaku.example.org:x` or `evil.example@kohaku.example.org`, two `Host` headers (`kohaku.example.org`, `evil.example`), or none
- **THEN** each response is 421

#### Scenario: Attacker relies on forwarded headers
- **WHEN** an attacker sends `GET /` with `Host: evil.example` and `X-Forwarded-Host: kohaku.example.org`, and `GET /` with `Host: kohaku.example.org`, `X-Forwarded-Host: evil.example` and `Forwarded: host=evil.example;proto=http`
- **THEN** the first gets 421 and the second is served exactly as without those headers

### Requirement: In-memory host map
Kohaku SHALL choose the router from an in-memory host map, without a database query:
- the main host (`KOHAKU_BASE_URL`'s host, lowercased, without port) maps to the main router
  whatever port `Host` names; names that contain it, subdomains included, are not the main host;
- each project host (configured for exactly one project) maps to the project router bound to
  that project; only the `projects` capability adds them, so this change maps only the main host.

#### Scenario: Attacker uses lookalike or random hosts
- **WHEN** an attacker sends `Host` `evil.kohaku.example.org`, `kohaku.example.org.evil.example`, `bugs.example.net` (no project host in this change), then 10 000 random values
- **THEN** every response is 421 and no request performs a database query

### Requirement: Unknown hosts get 421
A request whose normalized host is not in the host map SHALL receive `421 Misdirected Request`
for every method and path except the two internal endpoints. It SHALL NOT reach any router, be
redirected or set a cookie; the 421 body SHALL be the same for every unknown host and never
contain the `Host` value.

#### Scenario: Attacker sends an unknown host
- **WHEN** an attacker sends `GET /`, `GET /admin`, `POST /` and a GET of the landing page's stylesheet with `Host: evil.example`
- **THEN** each response is 421 with a body that lacks `evil.example` and equals the body for `Host: other.example`

#### Scenario: Internal service name and IP literals are unknown hosts
- **WHEN** `GET /` arrives with `Host: kohaku:8080` and with `Host: 203.0.113.7`
- **THEN** each response is 421

### Requirement: Internal endpoints are matched before host routing
Exactly two paths, `/healthz` and `/.well-known/kohaku/tls-ask`, SHALL be matched by exact path
before host routing, for every `Host` including unknown hosts, IP literals and none. They SHALL
never answer 421; they SHALL answer GET and HEAD, and 405 to any other method. No other path,
including others under `/.well-known/kohaku/`, SHALL be matched before host routing. They SHALL
belong to no rate-limit class: no token, no 429, no client-address resolution. The `operations`
capability specifies the `/healthz` response.

#### Scenario: Probes with internal Host values
- **WHEN** `kohaku healthcheck` sends `GET /healthz` with `Host: 127.0.0.1:8080`, and the reverse proxy sends a TLS ask with `Host: kohaku:8080`
- **THEN** the endpoints answer, neither status is 421, and no client-address warning is logged

#### Scenario: Attacker exceeds the read budget on internal endpoints
- **WHEN** an attacker sends 1 000 requests to each internal endpoint within one minute from one IPv4 address (`read` allows 300 per minute)
- **THEN** no response is 429, and a following `GET /` on the main host from that address is not rate limited

#### Scenario: Attacker probes around the internal endpoints
- **WHEN** an attacker sends `GET /.well-known/kohaku/other` and `GET /healthz/x` with `Host: evil.example`, and `POST` to each endpoint
- **THEN** the GETs get 421 and the POSTs get 405

### Requirement: TLS ask answers only for configured project hosts
`GET /.well-known/kohaku/tls-ask` SHALL answer 200 only when the query string has exactly one
`domain` parameter whose decoded value, normalized as `Host` is, is a project host in the host
map; otherwise 404, including for the main host, an unknown name, and a missing, empty or
repeated `domain` (so every name in this change). It SHALL decide from the in-memory map alone,
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
- **WHEN** an attacker sends 10 000 TLS asks to an instance running this change, each for a different name
- **THEN** every response is 404 with the same body, none queries the database, and the logs hold no per-request line and no queried name

### Requirement: Main-host routes
The main router SHALL serve only the landing page at `/` and the assets under `/static/`; every
other path gets 404, including `/admin`, `/admin/*` and `/p/*`. Only the main host SHALL serve
the admin area and the login, invite, reset, setup and unsubscribe pages once later capabilities
add them. Routes of a `dev` build (distribution capability) SHALL be absent without the `dev`
feature, on this router and on project routers. The landing page SHALL be 200 HTML, identical
whatever the query string or headers, with no project, report, user or configuration data (no
project names, slugs, hosts or counts, no configured values) and no cookie. It SHALL change no
state: a `POST /` that passes every `http-security` and `request-limits` check (Fetch-Metadata
and Origin, multipart, body cap) gets 405.

#### Scenario: Attacker probes for the admin area
- **WHEN** an attacker requests `GET /admin`, `/admin/login`, `/p/demo` and `/p/demo/api/v1/reports` on the main host
- **THEN** each response is 404, and `/admin`'s status and body equal those of `GET /does-not-exist`

#### Scenario: Attacker tries to reflect input into the landing page
- **WHEN** an attacker requests `GET /?q=<script>alert(1)</script>` on the main host with `X-Forwarded-Host: evil.example`
- **THEN** it gets 200, `Content-Type: text/html; charset=utf-8`, no `Set-Cookie`, no project listed, and the body of a plain `GET /`

#### Scenario: POST to the landing page
- **WHEN** a client sends `POST /` on the main host with `Origin: https://kohaku.example.org` matching `KOHAKU_BASE_URL`
- **THEN** the response is 405

### Requirement: Project-host routes
A project-host request SHALL be served by the project router bound to that host's project, which
has no path parameter selecting a project and restricts every data lookup to that project. `/p`,
`/p/*`, `/admin`, `/admin/*` and the login, invite, reset, setup and unsubscribe pages SHALL get
404 there. In this change it SHALL serve only the assets under `/static/` (plus `dev`-build routes);
every other path gets 404.

#### Scenario: Attacker addresses another project or the admin area through a project host
- **WHEN** `bugs.example.net` maps to project A and an attacker requests `/p/b`, `/p/b/api/v1/reports`, `/admin` and `/admin/login` there
- **THEN** each response is 404 and none contains data of project B

### Requirement: Hashed static assets on both routers
Kohaku SHALL serve its static assets under `/static/` with identical bytes on the main host and
every project host. Each asset path SHALL contain a hash of its content, and pages SHALL reference
assets only by these paths, except `/static/dev-reload.js`, which only a `dev` build serves and
references. Stylesheets SHALL be served as `text/css`, scripts as `text/javascript`. Without the
`dev` feature the asset set is fixed at build time: a `/static/` path naming no asset in it,
including an outdated hash, SHALL get 404, and no `/static/` request SHALL read the filesystem.

#### Scenario: Stylesheet on both hosts
- **WHEN** a client requests the landing page's stylesheet on the main host and on project host `bugs.example.net`
- **THEN** both responses are 200 with `Content-Type` `text/css` and identical bodies

#### Scenario: Attacker requests stale or traversal paths
- **WHEN** an attacker requests the stylesheet with one hash character changed, `/static/../../data/kohaku.db`, `/static/%2e%2e%2f%2e%2e%2fdata%2fkohaku.db` and `/static/`
- **THEN** each response is 404 and no file is read from the filesystem

### Requirement: Absolute URLs come only from configured hosts
Every absolute URL Kohaku emits (pages, JSON bodies, `Location` headers, mail, CLI output) SHALL
be built from configuration only, never from the request's `Host`, `X-Forwarded-Host`,
`X-Forwarded-Proto` or `Forwarded` header or its request target. The main-host origin is exactly
the scheme, host and port of `KOHAKU_BASE_URL`; a project host's is `https://` plus that host.
Token-bearing (setup, invite, reset) and unsubscribe links SHALL always use the main-host origin.

#### Scenario: Attacker forges Host and forwarded headers
- **WHEN** `KOHAKU_BASE_URL` is `https://kohaku.example.org:8443` and an attacker sends `Host: kohaku.example.org:9999`, `X-Forwarded-Host: evil.example`, `X-Forwarded-Proto: http`
- **THEN** every absolute URL built for that request starts with `https://kohaku.example.org:8443/`

#### Scenario: Token links from a project-host request use the main host
- **WHEN** a token-bearing or unsubscribe link is built for a request on project host `bugs.example.net`
- **THEN** the link starts with the `KOHAKU_BASE_URL` origin

### Requirement: Cookies only from main-host handlers with the __Host- prefix
Kohaku SHALL set cookies only in main-router responses, each named `__Host-…` with `Secure`,
`Path=/` and no `Domain`. Project routers, 421 responses and internal endpoints SHALL NOT send
`Set-Cookie`; state project-host pages need (proof-of-work challenges, one-time-code
verification ids, verified-email tokens) SHALL travel in form fields or JSON bodies, never in
cookies. No response in this change SHALL contain `Set-Cookie`.

#### Scenario: No cookies in this change, even for a forged one
- **WHEN** a client requests every route of this change on the main host, on project host `bugs.example.net` with `Cookie: __Host-kohaku_session=forged`, on an unknown host and on both internal endpoints
- **THEN** no response contains `Set-Cookie`
