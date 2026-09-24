# Tasks

- [ ] 1.1 Add `migrations/0010_plus_one.sql` (D1); verify the §12 migration test.
- [ ] 1.2 Add the epoch key, rotation and cleanup (D2); verify rotation with injected time and
  "Backup reveals no voters".
- [ ] 1.3 Add the `plus_one` class, PoW purpose and vote routes (D2, D3; +1: Counting;
  request-limits); verify "Attacker inflates a count", "Closed reports take no votes" and the
  class scenario.
- [ ] 1.4 Add count display and `sort=wanted` (plus-one: Showing counts and most wanted;
  moderation); verify "Most wanted", 400 for unknown `sort`, and nothing shown with +1 off.
- [ ] 1.5 `docs/api.md` and README; final check: the before-finishing commands.
