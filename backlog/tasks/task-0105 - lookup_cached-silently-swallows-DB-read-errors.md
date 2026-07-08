---
id: TASK-0105
title: lookup_cached silently swallows DB read errors
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:11'
updated_date: '2026-07-08 17:24'
labels:
  - robustness
  - observability
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/resolver.rs:131-148. lookup_cached pattern-matches Ok(Some(ap)) / Ok(None) / Err(e) where the Err path only warn-logs and continues. A real DB error (corruption, disk full, lock contention) returns the same shape as 'not in cache'. handle_locate then treats the BSSID as a miss, kicks off background resolver, burns API quota refetching what's actually still in the DB. db_write_failures counts writes, not reads, so this is invisible to ops.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Add db_read_failures: u64 counter to DaemonState.stats (parallel to db_write_failures); expose via /stats
- [x] #2 lookup_cached increments db_read_failures on Err, OR returns a typed result the caller can inspect
- [x] #3 Unit test: simulate a DB lock-busy or query error; assert the counter increments
- [x] #4 Whereami stats CLI line shows the new counter
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. DaemonState: add db_read_failures Arc<AtomicU64> field + record_db_read_failure() helper (server.rs)
2. resolver.rs: call state.record_db_read_failure() in both lookup_cached and lookup_cached_readonly Err arms (behavior otherwise unchanged)
3. StatsResponse (server.rs) + handle_stats: surface db_read_failures
4. Init to 0 in both constructors: main.rs real init + server.rs test_state_with_db
5. Client: whereami-client lib.rs StatsResponse (#[serde(default)]) + main.rs render line
6. Tests: client lib.rs stats_wire JSON fixtures (both current+legacy), server.rs stats_wire assertion, and AC#3 deterministic regression (DROP TABLE aps -> lookup_cached -> assert counter>=1 & empty Vec). Assert on counter, not logs (task-0248 flaky).
7. Gate: nix develop -c just fmt && nix develop -c just e2e; run new tests twice.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented db_read_failures counter mirroring db_write_failures end-to-end. Commit 92047b4 (code only; tracker excluded).

Sites touched:
- whereamid/src/server.rs: DaemonState field + record_db_read_failure() helper, StatsResponse field, handle_stats load+populate, test_state_with_db init, stats_wire test assertion.
- whereamid/src/resolver.rs: increment in BOTH lookup_cached and lookup_cached_readonly Err arms; added AC#3 regression test.
- whereamid/src/main.rs: real-init Arc::new(0).
- whereami-client/src/lib.rs: StatsResponse field with #[serde(default)] (old-daemon JSON without the key parses to 0); both wire fixtures updated.
- whereami-client/src/main.rs: render_stats line + its render test literal.

Gotchas hit:
- TWO extra StatsResponse literals beyond the ones listed: whereami-client/src/main.rs:1540 render test (compiler flagged missing field). Fixed.
- serde-default: mirrored db_write_failures #[serde(default)] so legacy daemons parse.
- AC#3 deterministic trigger: lock_db(&state).execute_batch_for_test("DROP TABLE aps;") then call lookup_cached + lookup_cached_readonly; asserts counter goes 0->1->2 and Vec empty. Verified test_state shares underlying SQLite between test_db and DbHandle (same idiom as existing DROP TABLE last_fix autolocate test), so the reader thread sees the drop and returns Err. Asserted on the counter not logs (task-0248 tracing-capture flakiness).

Gate (nix develop -c just e2e): ALL GREEN. whereamid lib 192, whereamid bins 444 (incl new AC#3 test), proptests 12, shutdown 4, client lib 39, client bin 39, +integration. clippy --all-targets -D warnings clean. cargo fmt --check clean. New/focused tests run twice, pass both times.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added a db_read_failures observability counter so a real DB read error at the cache-lookup boundary is visible via /stats instead of masquerading as a cache miss (which silently burns API quota re-fetching resident rows). Mirrors the existing db_write_failures end-to-end.

Changes (commit 6be836f):
- DaemonState.db_read_failures: Arc<AtomicU64> + record_db_read_failure() helper (whereamid/src/server.rs), initialized in both the real (main.rs) and shared test (test_state_with_db) constructors.
- Incremented in the Err arm of BOTH lookup_cached and lookup_cached_readonly (whereamid/src/resolver.rs); warn! + empty-return otherwise unchanged.
- Surfaced via StatsResponse + handle_stats, and mirrored on the client: whereami-client StatsResponse.db_read_failures (#[serde(default)] for old-daemon compat) + render_stats CLI line.
- Tests: AC#3 regression lookup_cached_read_error_increments_db_read_failures_counter (DROP TABLE aps forces a real rusqlite read Err on both variants; asserts the atomic counter 0->1->2, NOT log capture — avoids the task-0248 tracing-capture flake); updated stats-wire tests both sides incl. the legacy-default path.

Verification: review gate qa-test-runner GO (735/735 workspace tests re-run, clippy -D warnings clean, fmt clean) + mped-architect GO. In-thread review hardening: tightened the record_db_read_failure() doc-comment which over-claimed "every site" (it covers only the cache-lookup entry points).

Scope/known limits (honest, AC-sanctioned): this is observability-only — an Err still returns empty and is treated as a miss, so handle_locate still over-fetches on a read error (root-cause control-flow fix filed as task-0253). The counter also does NOT yet cover the resolve_chain skip-cached/skip-not_found read-error swallow sites, which fall through to CALL providers (same bug class, filed as task-0252). db_read_failures:0 therefore means "no fault at the covered entry points", not "no DB read fault anywhere".
<!-- SECTION:FINAL_SUMMARY:END -->
