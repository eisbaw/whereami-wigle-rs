---
id: TASK-0258
title: 'not_found: optional --not-found-max-rows row-count cap'
status: To Do
assignee: []
created_date: '2026-07-08 20:02'
updated_date: '2026-07-08 20:08'
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
- [ ] #1 CLI flag --not-found-max-rows N added and plumbed through config
- [ ] #2 DB method evicts oldest not_found rows when count exceeds N
- [ ] #3 Called once per drain pass; test proves oldest-first eviction down to N
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
From task-0110 review (mped-architect F6): the age-based prune landed in 0110 stops FOREVER growth (dead rows past 2xTTL are deleted) but does NOT cap a BURST — a busy location (train station: ~1e4 transient MACs/day x 60d ~ 1e5-1e6 rows) accumulates unboundedly WITHIN the 2xTTL window. This task (row-count cap, oldest-evicted) is what actually delivers the "upper bound" the 0110 title promised. So 0110 only partially met its headline goal; this task completes it. Consider implementing alongside task-0259 (idx_not_found_checked_at) since both target the same not_found-at-scale problem.
<!-- SECTION:NOTES:END -->
