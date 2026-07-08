---
id: TASK-0252
title: >-
  Count (or consciously exempt) resolve_chain read-error swallow sites in
  db_read_failures
status: Done
assignee:
  - '@myself'
created_date: '2026-07-08 17:22'
updated_date: '2026-07-08 18:07'
labels:
  - robustness
  - observability
dependencies:
  - TASK-0105
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Follow-up from task-0105 review (mped-architect MEDIUM finding). task-0105 added db_read_failures but only wired record_db_read_failure() at the two cache-lookup ENTRY POINTS (lookup_cached / lookup_cached_readonly). The SAME "real DB read error masquerades as a miss and wastes API quota" bug class exists, UNCOUNTED, at:
- whereamid/src/resolver.rs ~305 (resolve_chain skip_cached short-circuit): `Err(e) => { warn!("db error looking up {bssid}: {e}"); None }` — after swallowing to None the chain FALLS THROUGH and CALLS the providers (Apple/WiGLE), directly burning quota. Arguably the MORE damaging instance.
- whereamid/src/resolver.rs ~322-325 (resolve_chain skip_not_found short-circuit): `.is_not_found(...).await.unwrap_or(false)` — a DB read error is swallowed to false ("not in not_found") -> chain proceeds to call providers. Same harm.
Lower-relevance cousins to consider: server.rs ~2451 debug_statuses().unwrap_or_default() (mislabels DB status on the debug surface) and server.rs ~3063 state.db.stats().await.unwrap_or_default() (a read error in the stats handler SILENTLY ZEROES cached_aps/pending_aps/not_found_aps/db_size_bytes — the row counts lie while db_read_failures stays 0).
Consequence: db_read_failures is currently "cache-lookup-entry-point read failures only", NOT "all swallowed DB read failures"; ops could see db_read_failures:0 during an active DB fault on the resolve path. Fix direction: route the two resolve_chain sites through state.record_db_read_failure() (and decide on the stats.unwrap_or_default row-count lie), OR consciously document them as distinct and out of scope. Update the record_db_read_failure() doc-comment accordingly when done. Deterministic to test (DROP TABLE aps / not_found and drive resolve_chain).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 resolve_chain skip_cached and skip_not_found read-error swallow sites increment db_read_failures (with a warn)
- [x] #2 Other swallowed DB-read sites reachable after DaemonState (pending drain get_pending, handle_stats db.stats, debug_statuses) also increment the counter; loud-failing reads that surface an error response are deliberately not counted
- [x] #3 Unit tests prove the resolve_chain and pending read-error sites bite (drop the table to force a real Err, assert the counter increments)
- [x] #4 record_db_read_failure() doc-comment defines the metric (swallowed/invisible read failures) and its coverage boundary
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Resolved the task-0105 under-count finding: db_read_failures now covers every swallowed/invisible DB read fault reachable after DaemonState exists, making db_read_failures:0 actually mean "no swallowed DB read fault" rather than "none at the two cache-lookup entry points".

Sites now counted (commit 5aba42b): resolver.rs resolve_chain skip_cached (was: swallow to None + fall through to CALL providers, burning quota) and skip_not_found (was: silent unwrap_or(false)); pending.rs drain_once get_pending (was: silent drain abort); server.rs handle_stats db.stats() (was: unwrap_or_default silently zeroing row counts) and debug_statuses (was: silent status mislabel). Metric defined as "swallowed" read failures — a read error that already surfaces an error response (get_last_fix on locate, get_fixes_in_range on history) is deliberately NOT counted because it is already visible; this mirrors db_write_failures (both count only warn-and-continue faults).

Control flow unchanged everywhere (skip_not_found still proceeds on a read error as before; only warn + count added). Tightened the record_db_read_failure() doc-comment to define the metric + coverage boundary (startup rehydration reads in main.rs predate the counter and cannot bump it).

Tests: 3 new deterministic regressions (resolve_chain skip_cached / skip_not_found / pending drain) that DROP the relevant table to force a real query Err and assert the counter goes 0->1, each isolated to one site. Review gate: qa-test-runner GO (738/738 workspace tests re-run, whereamid bin 447, clippy -D warnings clean, fmt clean, new tests pass twice) + mped-architect GO (7/7 swallowed runtime read sites covered, loud reads correctly excluded, tests confirmed to bite+isolate). In-thread review hardening: reordered the handle_stats db_read_failures snapshot to AFTER db.stats() so a stats-read fault is reflected in the SAME response; added the doc-comment startup-reads caveat.

Known limit / follow-up: the two reporting-surface increments (handle_stats, debug_statuses) are counted but not individually unit-tested (trivial one-line mirrors) — filed as task-0254.
<!-- SECTION:FINAL_SUMMARY:END -->
