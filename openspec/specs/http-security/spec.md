# http-security Specification

## Purpose
HTTP protections on every request and response of every host (design §7): security headers,
Cache-Control classes, cross-site write rejection, body caps, time limits and request tracing
that records no request data.

## Requirements

### Requirement: Security headers on every response

Kohaku SHALL itself, whatever the reverse proxy does, send each header below exactly once on
every response to a request that reached request handling (every host, 421 unknown-host
answers, `/healthz`, `/.well-known/kohaku/tls-ask`, every method and status, pre-routing
responses, every rejection of this capability); only a request the HTTP connection layer
cannot parse and a connection closed by the header read timeout are exempt. Apart from the
CSP, values SHALL be exactly these and SHALL replace any value a handler set.

- `Content-Security-Policy` as in "Exact Content Security Policy"
- `Referrer-Policy: same-origin` (never `no-referrer`), `X-Frame-Options: DENY`,
  `Cross-Origin-Opener-Policy: same-origin`, `X-Content-Type-Options: nosniff`, an empty
  `Permissions-Policy`, `Strict-Transport-Security: max-age=31536000` (no `includeSubDomains`,
  no `preload`)

#### Scenario: Every response carries the header set

- **WHEN** Kohaku answers `GET /` and `HEAD /` on the main host, `GET /healthz`, a tls-ask
  query, an attacker's `GET /` with unconfigured `Host: evil.example` (421), and rejections
  with 403 (Fetch-Metadata/Origin), 403 (admin isolation), 404, 405, 408, 413 and 415
- **THEN** each carries every listed header exactly once with exactly the listed value

### Requirement: Exact Content Security Policy

Every response SHALL carry exactly this policy unless its route sets its own under "Per-route
security declarations", on the main host, project hosts and `/admin` alike (separate hosts are
no reason to loosen it):

`default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'`

It SHALL NOT gain `'unsafe-inline'`, `'unsafe-eval'`, a nonce, a hash, a scheme source (such as
`data:`) or a host source, so pages SHALL contain no inline script, `<style>` element, `style`
attribute, event-handler attribute or `data:` URL. A `dev`-feature build SHALL add exactly
`connect-src 'self'` and change nothing else; other builds SHALL send exactly the policy above.

#### Scenario: Attacker-injected markup is not permitted by the policy

- **WHEN** an attacker gets an inline `<script>`, an `onerror` attribute, `<img src="data:…">`
  or `<img src="https://evil.example/pixel">` into any page, a 404, a 421 or `/healthz`
- **THEN** a non-`dev` build's policy is exactly the one above, which allows none of them and,
  with `X-Frame-Options: DENY`, forbids framing; a `dev` build adds only `connect-src 'self'`
- **AND** the landing page itself has no inline script, `<style>`, `style` or event-handler
  attribute and references only same-origin paths

### Requirement: Cache-Control classes

Every response SHALL carry exactly one `Cache-Control` with its route's class value, whatever
the status, unless its route sets its own under "Per-route security declarations"; `no-store`
SHALL win over any other applicable class. Outside the immutable class and route-set values,
no response SHALL carry `immutable`, `public` or a `max-age` above 0.

- `no-store`: `/admin` and `/admin/…`; login; pages reached through an invite, password-reset
  or setup token link; two-factor enrollment; anything rendered using a session; everything no
  other class covers (421, unmatched 404, `/healthz`, `/.well-known/kohaku/tls-ask`).
- `no-cache`: public HTML (here only the landing page), public JSON API, public screenshots,
  and every `/static/` response not serving a content-hashed asset.
- `public, max-age=31536000, immutable`: only a 200 to GET or HEAD serving a static asset whose
  path contains its content hash.

#### Scenario: Each response gets its class

- **WHEN** a client requests `GET /`, the landing page's hashed stylesheet, `/admin`,
  `/admin/login`, `/does-not-exist`, an unknown host, `/healthz` and the tls-ask endpoint
- **THEN** `GET /` gets `no-cache`, the stylesheet a 200 with the immutable class, the rest
  `no-store`
- **AND** an attacker's request for the stylesheet with one hash character changed gets
  `no-cache`, with no `immutable` and no `max-age`

### Requirement: Per-route security declarations

Every route on every host SHALL declare its Cache-Control class, body cap, deadline, multipart
acceptance and header-less exemption; defaults are the class from "Cache-Control classes",
65536 bytes, 15 s, multipart rejected, not exempt. A route SHALL deviate from a default or set
its own CSP or `Cache-Control` only where a requirement of its own capability states it; the
header layer SHALL keep that value, and a route's own CSP SHALL NOT allow any source, keyword
or directive value the release policy denies. No route of this change deviates; later
deviations are reserved in design §6 (Submission) and §7. The test suite SHALL enumerate every
registered route and fail when one lacks a declaration or its responses differ from it.

Rejections SHALL apply in this order, the first applicable one answering: unknown host 421
(host-routing) → admin resource isolation 403 → Fetch-Metadata/Origin 403 → rate limit 429
(request-limits) → multipart 415 → body cap 413 → the route (including 405); a request
unanswered at its deadline SHALL get 408. `/healthz` and `/.well-known/kohaku/tls-ask` are
matched before host routing and subject to none of these rejections.

#### Scenario: Route table matches its declarations

- **WHEN** every route of this change is requested on its host
- **THEN** each response carries exactly the release CSP and its class's `Cache-Control`
- **AND** a route without a declaration, or with an own CSP or `Cache-Control` no requirement
  of its capability names, fails the test suite

### Requirement: Fetch-Metadata and Origin check on state-changing requests

Every request routed to the main host or a project host whose method is not GET or HEAD
(OPTIONS and unknown methods included), on admin, public and unmatched paths alike, SHALL pass
this check before any handler runs, any body byte is read, and the multipart and body-cap
checks:

1. `Sec-Fetch-Site` present and not exactly `same-origin` → 403.
2. Else `Origin` present and not byte-for-byte the expected origin → 403 (`null` never matches).
3. Else, if both are absent → 403 unless the route is exempt.
4. Else pass.

- More than one `Sec-Fetch-Site` or `Origin` header → 403. A rejected request SHALL reach no
  handler, and its response SHALL carry no `Access-Control-Allow-*` header.
- The expected origin SHALL come only from the routed host entry: exactly `KOHAKU_BASE_URL`
  (scheme, host, optional port; browser form per configuration) on the main host,
  `https://{project host}` on a project host (arriving with the `projects` change). `Host`
  selects the entry but its text SHALL NOT enter the expected origin; `X-Forwarded-*`,
  `Forwarded` and every other header SHALL be ignored. Another configured host's origin SHALL
  NOT match.
- Only a JSON API write route or the one-click unsubscribe POST SHALL be exempt, and only where
  its own capability declares it; admin, login, invite, password-reset, setup and HTML-form
  routes and unmatched paths never are. Exemption covers only rule 3. None in this change.
- No route SHALL change state on GET or HEAD. `/healthz` and `/.well-known/kohaku/tls-ask` are
  outside the check and answer every other method 405 (host-routing), whatever the headers.

#### Scenario: Same-origin writes pass and internal endpoints are outside the check

- **WHEN** `POST /` reaches the main host (`KOHAKU_BASE_URL` `https://kohaku.example.org`) with
  that `Origin`, with and without `Sec-Fetch-Site: same-origin`; and an attacker sends
  `POST /healthz` with `Sec-Fetch-Site: cross-site` and `POST` tls-ask with neither header
- **THEN** each is answered 405 (the routes' own answer), not 403

#### Scenario: Attacker's page or script triggers a cross-site write

- **WHEN** an attacker sends to the main host `POST /` with `Sec-Fetch-Site` `cross-site`,
  `same-site` or `none` (with or without the matching `Origin`) or with two `Origin` or two
  `Sec-Fetch-Site` headers (one valid); cross-site `PUT`, `PATCH`, `DELETE`, `PROPFIND`, an
  `OPTIONS` preflight (`Access-Control-Request-Method: POST`) and a `POST` declaring
  `Content-Length: 60000` with no body; and `POST /`, `PUT /admin/anything` and
  `POST /does-not-exist` with neither header
- **THEN** each is 403 without any handler, body wait or `Access-Control-Allow-*` header
- **AND** an exempt route still answers 403 to `Origin: null` or `Sec-Fetch-Site: cross-site`

#### Scenario: Attacker sends a null, near-miss or spoofed Origin

- **WHEN** an attacker sends `POST /` to the main host, with and without
  `Sec-Fetch-Site: same-origin`, with `Origin` `null`, `https://evil.example`,
  `http://kohaku.example.org`, `https://kohaku.example.org:443`, `https://kohaku.example.org/`,
  `https://KOHAKU.EXAMPLE.ORG`, `https://kohaku.example.org.evil.example`; or with
  `Origin: https://evil.example` plus `X-Forwarded-Host: evil.example`,
  `X-Forwarded-Proto: http` and `Forwarded: host=evil.example;proto=http`; or with
  `Host: KOHAKU.EXAMPLE.ORG:8080` and `Origin: https://KOHAKU.EXAMPLE.ORG:8080`
- **THEN** each is 403, while the last with `Origin: https://kohaku.example.org` passes

#### Scenario: Attacker writes across configured hosts

- **WHEN** `KOHAKU_BASE_URL` is `https://kohaku.example.org:8443`, and POSTs arrive on the main
  host with `Origin: https://kohaku.example.org` or, from script on project host
  `bugs.example.net` (`Sec-Fetch-Site` `same-site` or absent), `https://bugs.example.net`, and
  on `bugs.example.net` with the main host's origin
- **THEN** each is 403, while each host's own origin (`:8443` included) with no
  `Sec-Fetch-Site` passes

### Requirement: Resource isolation for admin GET requests

GET and HEAD on the main host for `/admin` or `/admin/…` SHALL be checked before routing:
`Sec-Fetch-Site` absent, `same-origin` or `none` passes; `same-site` or `cross-site` passes
only with exactly one `Sec-Fetch-Mode: navigate`, else 403; any other value, or more than one
`Sec-Fetch-Site` header, gets 403.

#### Scenario: Attacker embeds, fetches or forges an admin request

- **WHEN** an attacker's page loads `/admin/anything` as an image (`cross-site`, `no-cors`),
  same-site script fetches it (`same-site`, `cors`), or `GET /admin` carries
  `Sec-Fetch-Site: evil` or both `same-origin` and `cross-site`
- **THEN** each is 403
- **AND** `GET /admin` with `Sec-Fetch-Site` `same-origin`, `none`, none at all, or
  `cross-site` with `Sec-Fetch-Mode: navigate` passes the check

### Requirement: Default request body cap of 64 KiB

Every route SHALL read at most 65536 bytes of a body, whatever the method or content type,
unless it raises its cap under "Per-route security declarations" (none here); no cap SHALL
exceed 26214400 bytes (25 MiB). A route that does not use the body SHALL NOT read it. A
`Content-Length` above the cap SHALL get 413 before any body byte is read, on every route and
method; a body without it SHALL get 413 once past the cap, and reading SHALL stop. Kohaku SHALL
buffer, and a handler SHALL receive, at most the cap. No route here reads a body; body-reading
scenarios use the test suite's own route table.

#### Scenario: Attacker declares or streams an oversize body

- **WHEN** an attacker sends a same-origin POST with `Content-Length: 65537` and no body bytes
  to `POST /` (which ignores its body) and to a body-reading route, and a 1 GiB chunked POST to
  the body-reading route
- **THEN** each is 413, the first two without waiting for a body byte, the last with at most
  65536 bytes buffered and reading stopped
- **AND** a 65536-byte body passes the cap

### Requirement: Multipart bodies rejected unless explicitly accepted

A request routed to the main host or a project host whose `Content-Type` media type is
`multipart` (any subtype, case-insensitive, with or without parameters) SHALL get 415 for
every method without any body byte read, unless its route accepts multipart (none here) or an
earlier rejection (such as 403 or 429) answers it.

#### Scenario: Attacker uploads multipart

- **WHEN** an attacker sends a same-origin `POST /` with `multipart/form-data; boundary=x`,
  `Content-Length: 26214400` and no body bytes; `POST /` with `Multipart/Form-Data`,
  `MULTIPART/MIXED` or `multipart/related`; and `GET /` with `multipart/form-data`
- **THEN** each is 415, without waiting for any body byte

### Requirement: Request deadline

Every request SHALL be answered within its deadline, counted from the end of its headers and
covering body reading and handling: 15 s on 64 KiB routes, 120 s on a route with a larger
declared cap (none here). At the deadline Kohaku SHALL answer 408, stop processing and release
every concurrency permit the request holds (request-limits). Scenarios needing a body-reading,
permit-taking or slow route use the test suite's own route table.

#### Scenario: Permit is released at the deadline

- **WHEN** an attacker trickles a declared 60000-byte body at one byte per second to a
  body-reading route holding a public write permit, or handling a complete request exceeds 15 s
- **THEN** each gets 408 15 s after the end of its headers, and the permit is free for the next
  public write right after

### Requirement: Request header read timeout

Kohaku SHALL close a connection without any response when a request's line and headers have
not all arrived within 10 s after it started reading that request, for every request on every
connection (later kept-alive requests included), however many bytes keep arriving.

#### Scenario: Attacker stalls or trickles headers

- **WHEN** an attacker sends `GET / HTTP/1.1` and one header line then nothing, and on 100
  other connections one header byte every 2 s
- **THEN** each connection closes within 10 s of its request's start with no response bytes

### Requirement: Request tracing records only route pattern and status

Every tracing record (span or event) for an HTTP request SHALL contain, from the request and
response, only the matched route pattern (parameters unexpanded) and the status, and no path
at all when no route matched: never the URI, concrete path, query, `Host`, other header values,
cookies, body or client address.

#### Scenario: Attacker puts personal data in the URL and headers

- **WHEN** at the most verbose level an attacker requests the stylesheet with
  `?email=alice@example.com&token=s3cret`, and `/alice@example.com/s3cret` (404), each with
  `Cookie: __Host-kohaku_session=c00kie`, `Authorization: Bearer t0ken`,
  `X-Forwarded-For: 203.0.113.7`, `User-Agent: agent-x1` and `Referer: https://evil.example/r3f`
- **THEN** records hold at most the static route pattern and status; no log output contains
  the query string or any of those values
