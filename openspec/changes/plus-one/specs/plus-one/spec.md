# Spec Delta

## Purpose

The "+1" counter: how anyone can say a public report affects them too, bounded to one
count per network per report per day without storing who counted, and how the count is shown
and used to find the most wanted reports (design §6 +1).

## ADDED Requirements

### Requirement: Counting

On a project with `plus_one_enabled`, an `open` or `in_progress` public report SHALL accept
"+1" through `POST {base}/r/{n}/plus-one` (a form with a solved `plus_one` proof of work
bound to the project and report number; `303` back to the report) and
`POST /api/v1/reports/{n}/plus-one` (`{"pow"}`, answering `{"plus_one_count": …}`). Each request
SHALL pass the `plus_one` rate-limit class and the proof of work, then take a public-write
permit and, in one write, add a vote row for the voter and increment the count only if the
voter has none for this report in the current epoch. The voter SHALL be identified only by an
HMAC of the report and the resolved client's IPv4 /32 or IPv6 /56 under a random key held in
memory and replaced every 24 hours (and at startup); vote rows of other epochs SHALL be
deleted at each replacement. A repeated vote SHALL get the same answer with the count
unchanged. Any other report, status or project SHALL get the public 404.

#### Scenario: Attacker inflates a count
- **WHEN** an attacker sends 50 valid "+1" posts for report 7 from one /56 within a day, and 50 more from 50 addresses inside one IPv6 /56
- **THEN** the count rises by exactly 1

#### Scenario: Backup reveals no voters
- **WHEN** an attacker with a backup and a candidate address wants to know whether it voted for report 7
- **THEN** no stored value lets them check, because the key never left memory

#### Scenario: Closed reports take no votes
- **WHEN** someone posts "+1" for a fixed report, a pending one and a report of a project with +1 off
- **THEN** each gets the public 404 and no count changes

### Requirement: Showing counts and most wanted

When `plus_one_enabled` is on, list rows and detail pages SHALL show each report's count and
the report page SHALL offer the "+1" button on open and in-progress reports; when off,
neither appears and the API's count is null. The Open and In progress tabs and their API
lists SHALL accept `sort=wanted`, ordering by count descending, then number descending, with
a keyset cursor of both; because counts change while paging, a report may then appear on two
pages or be skipped, which only this sort permits.

#### Scenario: Most wanted
- **WHEN** open reports 3, 5 and 9 have counts 12, 40 and 12 and a client requests the Open tab with `sort=wanted`
- **THEN** the order is 5, 9, 3
