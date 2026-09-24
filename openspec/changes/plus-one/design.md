# Design

## Decisions

**D1. Schema (`migrations/0010_plus_one.sql`).** `plus_ones(report_id → reports CASCADE,
voter_hash BLOB, epoch INTEGER, PRIMARY KEY (report_id, voter_hash))`; index
`(project_id, status, plus_one_count DESC, number DESC)` on `reports` for the sort.

**D2. Vote write.** `INSERT OR IGNORE INTO plus_ones …` then, only if a row was inserted,
`UPDATE reports SET plus_one_count = plus_one_count + 1 WHERE id = :id AND status IN
('open','in_progress') RETURNING plus_one_count`, one transaction through the public view
lookup. The epoch key (`kohaku/plus-one`, §5 "+1 epoch key") is replaced by the scheduler
every 24 h and at startup, followed by `DELETE FROM plus_ones WHERE epoch <> :current`.

**D3. PoW.** Purpose `plus_one` with the report number in the challenge (report-submission
D3); a lighter difficulty constant, measured like `submit`.

**D4. Deviation.** Votes only on open and in-progress reports (§6 is silent): a count on a
fixed or closed report asks for nothing.

## Risks / Trade-offs

- [One /56 may hold many households] → One count per network per day is the design's
  trade-off between abuse and fairness.
- [Wanted-sort paging can repeat or skip] → Stated in the spec; only this sort.
