---
id: TASK-0258
title: 'not_found: optional --not-found-max-rows row-count cap'
status: Done
assignee:
  - '@myself'
created_date: '2026-07-08 20:02'
updated_date: '2026-07-09 07:31'
labels:
  - robustness
  - db
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Deferred from task-0110. The age-based 2x-TTL prune (prune_not_found in db.rs, called per drain pass) now bounds the not_found table by time. This follow-up adds an OPTIONAL hard row-count cap: --not-found-max-rows N CLI flag + config plumbing; when the table exceeds N rows, evict the OLDEST (by checked_at). Only worth doing if the age-based prune proves insufficient at an extremely busy location where within-TTL churn alone blows the size budget. Requires: CLI arg, config field, a DB method deleting oldest rows above N (ORDER BY checked_at LIMIT), and a call site in drain_once.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 CLI flag --not-found-max-rows N added and plumbed through config
- [x] #2 DB method evicts oldest not_found rows when count exceeds N
- [x] #3 Called once per drain pass; test proves oldest-first eviction down to N
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add pub not_found_max_rows: Option<usize> to Args (config.rs) with #[arg(long)], no default (absence=None=no cap)
2. Add Database::cap_not_found_rows(&self, max_rows: usize) -> Result<usize> (sync) + DbHandle::cap_not_found_rows async wrapper (mirror prune_not_found). count first; if <= max no-op return 0; else DELETE oldest (count-max) rows ORDER BY checked_at ASC LIMIT ?1 (uses idx_not_found_checked_at)
3. In pending.rs drain, AFTER prune_not_found, if Some(n) call cap_not_found_rows(n), log evicted if >0
4. Tests: oldest-first eviction down to N + no-op case (count<=N)
5. Gate: nix develop -c just e2e, cargo fmt --check
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
From task-0110 review (mped-architect F6): the age-based prune landed in 0110 stops FOREVER growth (dead rows past 2xTTL are deleted) but does NOT cap a BURST — a busy location (train station: ~1e4 transient MACs/day x 60d ~ 1e5-1e6 rows) accumulates unboundedly WITHIN the 2xTTL window. This task (row-count cap, oldest-evicted) is what actually delivers the "upper bound" the 0110 title promised. So 0110 only partially met its headline goal; this task completes it. Consider implementing alongside task-0259 (idx_not_found_checked_at) since both target the same not_found-at-scale problem.

Implemented (In Progress, left for review):
- config.rs: not_found_max_rows: Option<usize> #[arg(long)], no default (None=no cap). TOML config is secrets-only so CLI flag is the required surface.
- db.rs: Database::cap_not_found_rows(max_rows) sync + DbHandle async wrapper. Counts first; no-op (return 0) if count<=max; else DELETE oldest (count-max) rows ORDER BY checked_at ASC (idx_not_found_checked_at, task-0259).
- pending.rs: after prune_not_found, guarded by if let Some(n) => cap_not_found_rows(n), info-logs evicted>0. Default-off byte-identical.
- Tests: cap_not_found_rows_evicts_oldest_by_checked_at_down_to_n; cap_not_found_rows_is_noop_when_count_at_or_below_cap.
Gate: nix develop -c just e2e PASS (894 passed, baseline 890 +2 tests counted under 2 configs), clippy -D warnings clean, cargo fmt --check clean.
Commit 09b1c45. Not pushed. ACs not checked, status left In Progress for review.

## Review gate (orchestrator, independent) — GO/GO + hardening
- Architecture: CONFIRMED default-off byte-identical (flag Option<usize>, no default -> None; drain call site `if let Some(max_rows)` skips the block entirely; cap is ADDITIVE after prune, prune/revive untouched). Eviction SQL correct: DELETE ... rowid IN (SELECT rowid ... ORDER BY checked_at ASC LIMIT excess); evicts oldest-by-checked_at; no-op when count<=max (excess=saturating_sub, <=0 -> Ok(0), incl. count==max boundary); no underflow. No-op boundary test bites the >= off-by-one. Scope = config.rs+db.rs+pending.rs only.
- MEDIUM finding (test-bite gap + lying comment) HARDENED in-thread (commit 05b6b41): the eviction test inserted rows with rowid/insertion order MATCHING checked_at order, so an ORDER BY rowid regression would PASS (test blind to checked_at-vs-rowid); the comment also falsely referenced a non-existent "reversed sibling". Fixed by DECOUPLING: insert newest-first (rowid 1=newest, rowid 7=oldest) with the bssid suffix tracking checked_at rank, and corrected the comment. MUTATION-VERIFIED: temporarily changing the SQL to ORDER BY rowid makes the test FAIL ("only the newest 3 must survive"), reverted. Now genuinely bites checked_at ordering. Full gate re-run green (894).
- QA (orchestrator in-thread): 894/894, clippy 0 -D warnings, fmt clean.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added an OPTIONAL hard row-count cap on the not_found table, complementing the age-based 2x-TTL prune (task-0110) for extremely busy locations where within-TTL churn could blow the size budget.

- CLI: --not-found-max-rows N (Option<usize>, no default -> None -> NO cap; behavior byte-identical to today when unset).
- DB: Database::cap_not_found_rows(max_rows) + async wrapper — no-op when count<=max, else DELETE the oldest (count-max) rows by checked_at ASC (uses idx_not_found_checked_at from task-0259). Returns the evicted count.
- Drain: called once per drain pass, AFTER prune_not_found, guarded by if let Some(n); logs the evicted count when nonzero.

Tests: oldest-first eviction down to N (hardened to insert rowid-order REVERSED from checked_at-order so it genuinely bites the checked_at ordering — mutation-verified: an ORDER BY rowid regression fails it) + the no-op boundary (count<N and count==N -> 0 evicted, bites the >= off-by-one).

Opt-in, default-off; additive after prune; no other not_found behavior changed. 894 workspace tests, dual gate GO/GO. Commits 09b1c45 (feature) + 05b6b41 (eviction-test bite hardening).
<!-- SECTION:FINAL_SUMMARY:END -->
