---
id: TASK-0096
title: 'Daemon-side periodic locate: write fixes timeseries autonomously'
status: Done
assignee:
  - task0096-implementer
created_date: '2026-05-19 06:53'
updated_date: '2026-07-05 15:13'
labels:
  - observability
  - history
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Today, the fixes table is populated ONLY when a client calls /locate. A user who runs 'whereami locate' once in the morning then moves around without re-querying leaves an empty history. Real incident: user has only 25 fixes over 7 days, all from manual locate invocations; most of the actual trajectory is missing.

Add a background autolocate loop in the daemon, parallel to the existing scan / pending-drain / history-prune loops (task-0075 cooperative-shutdown pattern). Default ENABLED at 120s interval (similar to last_fix already updated on every /locate; this is incrementally similar in privacy terms). 0 disables.

Edge cases:
- First fix ever (last_fix is NULL): always write — dedup check is skipped.
- Empty scan (no stable BSSIDs visible): skip; do not write a useless fix.
- No cached APs to trilaterate from: skip.
- Dedup: skip when last_fix is non-NULL AND haversine_m(new, last_fix) < history_segment_distance_m AND (now - last_fix.at) < autolocate_interval_secs.

This composes with task-0094 (the implied-speed and confidence fields populate every autolocate fix too, so the trajectory carries quality metadata) and task-0095 (raw history view becomes useful with denser fixes).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Add --autolocate-interval-secs <N> CLI flag (default 120; 0 disables); also expose as services.whereami.autolocateIntervalSecs in nix/module.nix and homeManagerModules
- [x] #2 Add run_autolocate_loop background task in main.rs alongside the scan/drain/prune loops; uses tokio::select! on DaemonState.shutdown like task-0075; max graceful-shutdown wait of 5s matches task-0075's existing budget
- [x] #3 Loop body extracts the locate logic into a shared internal function compute_locate(state) -> Result<LocateOutcome> called by both handle_locate and run_autolocate_loop; the autolocate path does NOT touch any TCP connection
- [x] #4 Skip-conditions checked in order: (a) no stable BSSIDs in debouncer -> skip; (b) no cached APs after debouncer filter -> skip; (c) last_fix is non-NULL AND haversine_m(new_pos, last_fix) < history_segment_distance_m AND age(last_fix) < autolocate_interval_secs -> skip-as-dedup. First-fix (last_fix NULL) bypasses dedup and writes unconditionally
- [x] #5 Add Stats counter autolocate_fixes_written: u64; wired into DaemonState.stats following the pattern of inflight and db_write_failures (task-0074/0076); exposed via /stats response and whereami stats CLI line
- [x] #6 Test (integration-style in whereamid/tests/): build a DaemonState with a test scanner trait impl that returns a fixed Vec<ScannedAp> with cached positions in aps table; set autolocate_interval_secs=1; spawn run_autolocate_loop; wait 3s; assert fixes table has >= 2 rows and autolocate_fixes_written counter is >= 2. Use tokio::time::pause + advance if the scanner is exercised purely in-process; otherwise sleep 3s real time
- [x] #7 Test: dedup — run autolocate twice with identical scanner output and no movement; assert second invocation does NOT write a new fix (counter unchanged, fixes table size unchanged)
- [x] #8 Test: first-fix — autolocate on empty fixes table writes the row even though it is the very first one
- [x] #9 README updated: brief paragraph in 'Caching and offline operation' or 'Architecture' covering autolocate; trade-off (richer history vs more disk writes) noted; --autolocate-interval-secs=0 documented as the off-switch
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add the unsigned autolocate interval configuration, shared NixOS/Home Manager option and emission, startup counter state, and additive daemon/client stats field with backward-compatible rendering and docs. 2. Extract the current locate pipeline into a structured shared core that accepts the original privacy ticket and an explicit Interactive or Autolocate purpose; preserve every interactive fallback, provider, plausibility, address, persistence, and JSON behavior. 3. Implement the autonomous policy inside that core: require stable latest-scan candidates, use cache only, trilaterate, serialize persisted-last-fix read/dedup/plausibility/persistence under privacy then last_fix ordering, and count only committed autonomous rows. 4. Add a delayed fixed-cadence main.rs loop with skipped missed ticks, fresh privacy ticket per pass, zero-disable behavior, cooperative cancellation, and the existing named-task five-second shutdown drain. 5. Add production-real in-binary tests with seeded Debouncer and real DB actors for cadence, skip order, first fix, dedup/boundaries/concurrency/failure/privacy, manual-regression behavior, stats, configuration, and Nix parity/bounds; update README and protocol docs. 6. Run mutation/focused tests, formatting, lint, Nix flag/default/bounds checks, full just e2e, freeze the diff, pass mandatory parallel QA and architecture review, then close and commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Phase 3 pre-implementation audit pinned the obsolete parts of the task. There is no scanner trait or ScannedAp: run_scan_loop remains the sole scanner owner and autolocate consumes the existing Debouncer snapshot. A literal whereamid/tests state fixture is impossible without widening binary-private server/config APIs, so AC6 is covered by integration-style in-binary async tests using server::test_state, a seeded ScanSample, and real DbRuntime actors. Tokio paused time does not advance Utc::now; the loop test uses an injected one-pass/clock seam so timer cadence and wall-clock dedup advance together without a flaky real sleep. The shared core signature must include the dispatch privacy ticket and an explicit purpose; the obsolete compute_locate(state)-only signature would weaken wipe epoch safety and cannot preserve the manual unstable-visible fallback while enforcing autonomous stable-only behavior. Interactive locate behavior and response bytes remain unchanged. Autolocate is stable-only and cache-only: no unstable fallback, Apple/WiGLE work, pending mutation, stale-last-fix response, or Nominatim scheduling; it may reuse an already-cached local address. It obtains a fresh privacy ticket every pass. Skip order is no stable eligible current candidate, then no eligible cached solver position, then autonomous dedup. The persisted last_fix row is authoritative. Dedup is autonomous-only and holds the same privacy -> last_fix -> DB serialization boundary as plausibility/persistence; it skips only when raw current-to-previous distance is strictly below history_segment_distance_m AND high-resolution nonnegative age is strictly below autolocate_interval_secs. Equality writes. A future previous timestamp warns and uses age zero. First fix bypasses dedup but still requires stable/cache/trilateration and a successful transaction. TASK-0094 last_fix winner, dedup, invalidated ticket, and failure append nothing and do not increment. autolocate_fixes_written means committed autonomous history rows since daemon start, excludes manual attempts, resets on restart, and is not privacy data cleared by wipe. Autonomous persistence updates memory only after DB success; failure warns, increments db_write_failures, leaves autonomous last_fix/counter unchanged, and the loop continues. Cache-read errors remain the separately tracked TASK-0105 debt; this cache-only path safely logs/skips and never triggers providers. The periodic task is not spawned at interval 0; otherwise first tick is delayed one interval, Tokio missed-tick behavior is Skip, passes never overlap or catch up in bursts, and the handle joins the existing single five-second named-task drain. The current fixes schema cannot carry TASK-0094 confidence fields; this task applies the same winner policy but does not claim durable quality metadata. TASK-0245 tracks the required schema/raw-history expansion. Debouncer snapshots can remain stale after scanner failure; changing the exact AC6 repeated-fixed-snapshot behavior needs a separately pinned freshness policy, tracked by TASK-0246 and documented as a limitation here. Additive stats remains protocol v1 and the typed client defaults the new field to zero for older daemons. Shared options.nix exposes autolocateIntervalSecs to both module types; zero is valid and default 120 parity is tested. No direct flake/module wrapper edit is needed.

Shutdown boundary: cancellation prevents the next pass while waiting for the interval and is checked immediately before admission. Once a pass starts, await it to completion rather than select-dropping an already-enqueued durable persistence await; otherwise SQLite could commit after the async memory/counter continuation was cancelled. The named task then observes cancellation and exits. Only the existing process-shutdown five-second drain may force-abort a wedged pass, after the control socket is closed and before DB actor shutdown.

Implementation completed after mandatory parallel QA and mped-architect review. The first architecture gate issued NO-GO for four root defects: autonomous consensus excluded visible unstable cached APs, an unbounded interval could panic only the background task, autonomous failures discarded their causes below debug, and Skip cadence was not mutation-pinned. The same sole writer corrected all four; final architecture and QA re-reviews returned GO on code hash c9a27b1c0a87795ebbb537d055b23b8fb4dc48cde29737b6fa908e8dd42bc93f. Final reviewed gates: exact 643-test e2e plus strict Clippy, formatting/diff checks, repeated focused concurrency/cadence/overflow/consensus/warning tests, 28-of-28 packaged flag parity, and shared Nix option bounds/default parity. Interval policy is 0 disabled or 1..31536000 seconds. TASK-0245 owns durable confidence metadata. TASK-0246 owns the documented retained-scan freshness risk and is the immediate release-blocking correctness follow-up before autonomous history should be described as fully trustworthy.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added daemon-side periodic cache-only locate collection, enabled every 120 seconds by default and disabled with interval 0. Interactive and autonomous paths share the privacy-safe locate/plausibility core while autonomous mode remains stable-only, provider-free, geocode-free, and connection-free. Autonomous writes are deduplicated with strict distance/time boundaries, commit before memory/counter publication, preserve monotonic last_fix, survive wipe/concurrency races, and expose a backward-compatible committed-row stats counter. Shared NixOS/Home Manager options, exact help/default/bounds parity, docs, and deterministic loop tests cover the complete behavior.
<!-- SECTION:FINAL_SUMMARY:END -->
