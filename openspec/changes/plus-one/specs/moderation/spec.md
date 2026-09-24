# Spec Delta

## MODIFIED Requirements

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
number, or by `sort=wanted` as the plus-one capability defines. A malformed cursor or unknown
`status`, `kind` or `sort` value SHALL get 400. Pages are
`no-cache` (http-security).

#### Scenario: Paging while reports change
- **WHEN** a client pages through 120 open reports while 3 new reports are approved and 2 listed ones move to fixed
- **THEN** no report appears twice across the pages it fetched and no report that stayed open is skipped

#### Scenario: Attacker sends crafted list parameters
- **WHEN** an attacker requests `?cursor=%00%ff`, `?cursor=` plus 10 KiB of base64url, `?status=pending`, `?status=spam` and `?kind=secret`
- **THEN** each gets 400 and no private report is listed
