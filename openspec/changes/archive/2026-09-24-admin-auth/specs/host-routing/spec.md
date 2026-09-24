# Spec Delta

## MODIFIED Requirements

### Requirement: Main-host routes
The main router SHALL serve only the landing page at `/`, the assets under `/static/` and the
`/admin` routes of the admin-auth capability (signed-in home, login, logout, account and
password reset), which also defines how `/admin` paths answer without a session; every other
path gets 404, including `/p/*`. Only the main host SHALL serve the admin area and the login,
invite, reset, setup and unsubscribe pages. Routes of a `dev` build (distribution capability) SHALL be absent without the `dev`
feature, on this router and on project routers. The landing page SHALL be 200 HTML, identical
whatever the query string or headers, with no project, report, user or configuration data (no
project names, slugs, hosts or counts, no configured values) and no cookie. It SHALL change no
state: a `POST /` that passes every `http-security` and `request-limits` check (Fetch-Metadata
and Origin, multipart, body cap) gets 405.

#### Scenario: Attacker probes for the admin area
- **WHEN** an attacker without a session requests `GET /admin`, `/admin/users`, `/p/demo` and `/p/demo/api/v1/reports` on the main host
- **THEN** both `/admin` paths get the same `303` to `/admin/login`, and both `/p/` paths get 404 with the status and body of `GET /does-not-exist`

#### Scenario: Attacker tries to reflect input into the landing page
- **WHEN** an attacker requests `GET /?q=<script>alert(1)</script>` on the main host with `X-Forwarded-Host: evil.example`
- **THEN** it gets 200, `Content-Type: text/html; charset=utf-8`, no `Set-Cookie`, no project listed, and the body of a plain `GET /`

#### Scenario: POST to the landing page
- **WHEN** a client sends `POST /` on the main host with `Origin: https://kohaku.example.org` matching `KOHAKU_BASE_URL`
- **THEN** the response is 405

### Requirement: Cookies only from main-host handlers with the __Host- prefix
Kohaku SHALL set cookies only in main-router responses, each named `__Host-…` with `Secure`,
`Path=/` and no `Domain`. Project routers, 421 responses and internal endpoints SHALL NOT send
`Set-Cookie`; state project-host pages need (proof-of-work challenges, one-time-code
verification ids, verified-email tokens) SHALL travel in form fields or JSON bodies, never in
cookies. The only cookies SHALL be `__Host-kohaku_session` and `__Host-kohaku_device`
(admin-auth), set only by a successful login and expired only by logout; no other response
SHALL contain `Set-Cookie`.

#### Scenario: Cookies only from login and logout, even for a forged one
- **WHEN** a client requests every route on the main host (a failed login included), on project host `bugs.example.net` with `Cookie: __Host-kohaku_session=forged`, on an unknown host and on both internal endpoints, then logs in successfully and logs out
- **THEN** only the successful login and the logout responses contain `Set-Cookie`, each cookie named `__Host-kohaku_…` with `Secure`, `HttpOnly`, `SameSite=Strict`, `Path=/` and no `Domain`
