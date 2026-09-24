# Spec Delta

## Purpose

What maintainers do with reports and what the public sees: the status lifecycle, single and
bulk moderation, audited edits, notes, deletion, and the public lists, detail pages and read
API with their Fixed / Implemented section (design §4 Lifecycle and Visibility, §6 Public
reads, §8 Moderation and Pages).

## ADDED Requirements

### Requirement: Status transitions

Only these transitions SHALL exist, each done by a maintainer of the project or the admin:

| From | To |
|---|---|
| pending | open (approve), spam (reject) |
| open | in_progress, fixed, closed |
| in_progress | open, fixed, closed |
| fixed, closed | open (reopen) |
| any status | hidden |
| hidden | open (unhide) |

Every transition SHALL be one conditional write on the report's current status, set the
status-change time, clear the source key when leaving `pending`, and record an audit entry in
the same transaction. Approving SHALL set the first-approval time; unhiding a report never
approved counts as its approval. Closing SHALL require a reason of 1–200 Unicode scalar values,
single line, shown publicly; leaving `closed` clears it. Rejecting as spam SHALL delete the
report's screenshots at once; spam reports SHALL be deleted 30 days after rejection. Hidden
reports SHALL never be purged and SHALL NOT count toward pending caps. Any other transition
SHALL be refused (409) without change.

#### Scenario: Stale or forged transitions
- **WHEN** two maintainers approve and reject the same pending report at once, and a forged form moves a `fixed` report to `in_progress`
- **THEN** exactly one of the first two takes effect, the other gets 409, and the forged transition gets 409 without change

#### Scenario: Spam is purged
- **WHEN** a report is rejected as spam and 30 days pass
- **THEN** the report and everything stored for it are gone, and its number is not reused

### Requirement: Maintainer pages and actions

Under `/admin/p/{slug}/` (grant required, users-and-invites), a maintainer SHALL see the
project's reports by status (pending first, spam and hidden included), open a report with its
full body, source grouping for pending reports ("N pending from this network", never the
network itself) and its notes, and:

- approve, reject as spam, change status, close with a reason, reopen, hide and unhide;
- edit the title and body, and change the kind between bug and feature (to feature only
  while the project accepts feature requests), each audited;
- add, edit and delete public notes (Markdown, at most 20 480 bytes), whose author is
  recorded only in the audit log;
- reject selected pending reports, all pending reports from one report's source, or all
  pending reports of the project, after a confirmation page stating the count.

Only the admin SHALL delete a report permanently, which removes its notes, screenshots and
queued mail. Every action form SHALL carry the CSRF token; every lookup SHALL bind the project
from the guard and the report by its number within it, never by a global id. While a report is
pending, its body and notes SHALL render with inert links. `/admin` SHALL show per project the
counts of pending, open and in-progress reports.

#### Scenario: Maintainer reaches into another project
- **WHEN** a maintainer granted only project A posts an approval to `/admin/p/a/r/5` where 5 exists only in project B, and requests `/admin/p/b/r/5`
- **THEN** the approval gets 404 and changes nothing, and the page gets 404

#### Scenario: Bulk reject by source
- **WHEN** 40 pending reports came from one network and 3 from others, and a maintainer confirms "reject all pending from this source" on one of the 40
- **THEN** those 40 become spam with one audit entry each, the other 3 stay pending, and no page shows the network

#### Scenario: Maintainer cannot hard-delete
- **WHEN** a maintainer posts a permanent deletion with a valid CSRF token
- **THEN** it gets 404 and the report remains

### Requirement: Visibility

Public means status `open`, `in_progress`, `fixed` or `closed`. Every public page, public API
response and public screenshot SHALL read reports and notes only through database views that
apply this rule, and serialize only public fields: number, kind, title, status, close reason,
+1 count (null when the project has +1 off), creation and status-change times, the
report's URL, and on detail the Markdown body, its rendered HTML and the notes (body,
rendered HTML, time; no author). A pending, spam, hidden or nonexistent report SHALL get the
same 404.

#### Scenario: Attacker probes private reports
- **WHEN** an attacker requests the page and API detail of a pending, a spam, a hidden and a nonexistent report number
- **THEN** all eight responses are 404 with identical bodies per route kind, and no list or count reveals them

### Requirement: Public pages and read API

Relative to the project base:

- `GET /` SHALL list public reports with tabs Open (default), In progress, Closed and Fixed
  (labelled "Fixed / Implemented" when the project accepts feature requests or has any public
  feature request, else "Fixed"), 50 per page, titles only, each row showing its number,
  status and, under the same condition, a visible kind label ("Bug" or "Feature") styled
  distinctly; `?kind=bug|feature` SHALL filter any tab.
- A fixed report SHALL be labelled "Implemented" when it is a feature request, "Fixed" when a
  bug; the stored status is `fixed` for both.
- `GET /r/{n}` SHALL show the report with its rendered body and notes.
- `GET /api/v1/reports?status=…&kind=…&cursor=…` and `GET /api/v1/reports/{n}` SHALL return
  the public fields as JSON, with `Access-Control-Allow-Origin: *`.

Lists SHALL use keyset pagination with a base64url `cursor` of the last row's key: open, in
progress and closed by number descending; fixed by status-change time descending, then
number. A malformed cursor or unknown `status` or `kind` value SHALL get 400. Pages are
`no-cache` (http-security).

#### Scenario: Paging while reports change
- **WHEN** a client pages through 120 open reports while 3 new reports are approved and 2 listed ones move to fixed
- **THEN** no report appears twice across the pages it fetched and no report that stayed open is skipped

#### Scenario: Attacker sends crafted list parameters
- **WHEN** an attacker requests `?cursor=%00%ff`, `?cursor=` plus 10 KiB of base64url, `?status=pending`, `?status=spam` and `?kind=secret`
- **THEN** each gets 400 and no private report is listed
