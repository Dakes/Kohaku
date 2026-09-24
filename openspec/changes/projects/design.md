# Design

## Context

Builds on `foundation` (host map in a `RwLock<Arc<_>>`, tls-ask, project router bound to a
project id) and `admin-auth` (sessions, CSRF, `Public`/`Session` access). Design §6 Domains
is the source.

## Goals / Non-Goals

**Goals:** a host change is live when it commits; nothing in routing queries the database
per request.

**Non-Goals:** see proposal.

## Decisions

**D1. Schema (`migrations/0003_projects.sql`).** `projects`: `id INTEGER PRIMARY KEY
AUTOINCREMENT`, `slug TEXT UNIQUE` with a `GLOB` check, `name`, `public_host TEXT UNIQUE`
(NULL allowed), the four switches as `INTEGER CHECK (x IN (0,1))` with defaults 0, 0, 0, 1,
`privacy_notice`, `security_contact`, `next_number INTEGER DEFAULT 1`, `created_at`. Later
tables reference it `ON DELETE CASCADE`, so deleting a project deletes its data.

**D2. Access level `Admin`.** The route table's access declaration gains `Admin`: the
session guard, then `role = admin`, else the same 404 as the `/admin` fallback. Every
project-administration route declares it; the matrix asserts maintainer 404s.

**D3. Routes.** `GET /admin/projects` (list and create form), `POST /admin/projects`,
`GET`/`POST /admin/p/{slug}/settings`, `POST /admin/p/{slug}/delete`. All `no-store`,
64 KiB, CSRF, no rate class (session-bound). Slug lookups bind the slug; an unknown slug is
the same 404.

**D4. Host map updates.** One function builds the map from `SELECT id, public_host FROM
projects WHERE public_host IS NOT NULL` plus the main host, and swaps the `Arc`. Admin
handlers call it after their transaction commits. The watcher connection (a dedicated
read-only connection, not one of the 4 readers) polls `PRAGMA data_version` every 2 s and
rebuilds when it changes; a failed rebuild keeps the old map and logs once per minute.

**D5. Canonical redirect.** A main-router route `/p/{slug}` and `/p/{slug}/{*rest}` for GET
and HEAD: look the slug up in an in-memory `slug → public_host` table built with the host
map (no query per request); if the project has a host and `rest` does not start with `api/`
answer 308, else fall through to the routes later changes add (404 now). The route sets its
own `Cache-Control: max-age=3600` (a declared deviation, http-security).

**D6. Text rules.** One `single_line` validator (Cc, U+2028/9 and the listed format
characters) shared with later title and close-reason validation; `privacy_notice`
normalizes CR LF and CR to LF first.

**D7. CLI.** `project create` parses `--name` and optional `--host` in that order; it runs
without tokio like `admin unlock`, with the keycheck, and relies on D4's watcher to reach a
running server.

**D8. Deviations from the design.**

1. **Slug is immutable** (§4 is silent): links, API clients and the redirect depend on it.
2. **Deleting a project deletes its reports** (§15 "CRUD"): confirmation by typed slug.
3. **`plus_one_enabled` defaults to on**, the other switches to off (§4 names only
   `screenshots_enabled` off).
4. **Order swapped with `users-and-invites`** (§15): grants reference projects.

## Risks / Trade-offs

- [A deleted project cannot be recovered without a restore] → Typed-slug confirmation;
  README says so; pre-migration copies and backups keep it.
- [Watcher polls every 2 s] → One cheap PRAGMA on an idle connection.
- [A browser cached the 308 for an hour after a domain move] → The old host gets 421 and the
  cache expires within 3600 s (§6).

## Migration Plan

Migration 3 adds `projects`; nothing existing changes. Rollback before a release: revert.
