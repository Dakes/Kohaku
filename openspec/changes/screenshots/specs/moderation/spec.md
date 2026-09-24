# Spec Delta

## MODIFIED Requirements

### Requirement: Visibility

Public means status `open`, `in_progress`, `fixed` or `closed`. Every public page, public API
response and public screenshot SHALL read reports and notes only through database views that
apply this rule, and serialize only public fields: number, kind, title, status, close reason,
+1 count (null when the project has +1 off), creation and status-change times, the
report's URL, and on detail the Markdown body, its rendered HTML, the screenshot URLs and
the notes (body, rendered HTML, time; no author). A pending, spam, hidden or nonexistent report SHALL get the
same 404.

#### Scenario: Attacker probes private reports
- **WHEN** an attacker requests the page and API detail of a pending, a spam, a hidden and a nonexistent report number
- **THEN** all eight responses are 404 with identical bodies per route kind, and no list or count reveals them
