---
id: TASK-0248
title: >-
  De-flake task-0239 tracing-capture test
  (locate_skips_non_finite_cached_ap_and_warns_with_bssid) — ~1-in-8 under
  parallel load
status: Done
assignee:
  - '@myself'
created_date: '2026-07-07 23:24'
updated_date: '2026-07-08 08:36'
labels:
  - robustness
  - test-flake
  - tech-debt
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The e2e gate has a pre-existing intermittent flake surfaced by the task-0230 QA review. Test: server::dispatch_tests::locate_skips_non_finite_cached_ap_and_warns_with_bssid (whereamid/src/server.rs ~6228), added by task-0239 (commit 1cec12e). It captures a WARN via `dispatch_command(...).with_subscriber(captured).await` (tracing WithSubscriber task-local dispatcher) and asserts exactly one WARN was seen. Reproduction (QA): passes 10/10 in isolation and 6/6 on a full-bin re-run, but fails ~1 in ~8 full-parallel `just e2e` runs with an EMPTY capture ([]) — the warning is lost, not duplicated. The warn is emitted INLINE/synchronously in handle_locate (server.rs:1903), the test is #[tokio::test] (current-thread, no spawns in the locate path), so the loss is not a background-task emission; the likely mechanism is tracing's task-local/global dispatcher resolution when cargo runs many #[tokio::test] fns concurrently on separate harness threads in one process (WithSubscriber/get_default interaction). This is a TEST-HARNESS defect, not a product bug (the non-finite skip itself is correct and separately asserted by the byte-level cache/sources assertions in the same test). Fix direction to evaluate: replace the per-future WithSubscriber capture with a more robust per-test subscriber (e.g. a dedicated CaptureLayer set as the global default guarded by a process-wide test mutex, or a current-thread with_default guard that provably spans the synchronous emission), or split the WARN-capture assertion into a deterministic unit around the emitting loop. Do NOT weaken the assertion to hide the loss. Verify the fix with an N-times (>=50) parallel-load loop, not a single sample.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Root cause of the empty-capture flake is identified and fixed at the harness level (not by weakening the WARN assertion)
- [x] #2 The test passes >=50/50 under full-parallel just e2e load (measured, not a single sample)
- [x] #3 The non-finite-skip WARN behavior is still positively asserted (the guard still bites)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
ROOT CAUSE (verified against tracing-core 0.1.36 source):
- Process-global per-callsite Interest cache. rebuild_callsite_interest folds every REGISTERED dispatcher's register_callsite via Interest::and and falls back to Interest::never() when the fold is empty.
- Thread-local with_default/with_subscriber dispatchers ARE registered into the global DISPATCHERS registry (Dispatch::new -> register_dispatch), but a callsite exercised by only ONE scoped test (the handle_locate non-finite warn!) can be first-registered in a window where that dispatcher is not folded in -> cached never -> warn! short-circuits -> event dropped -> capture sees [].
- The two prior fixes failed because they only touched the thread-local CAPTURING subscriber, never a PERMANENT registry member the fold reads.

FIX (candidate b, algebraic not probabilistic): install once a global default subscriber (set_global_default -> leaked, permanent DISPATCHERS member) whose register_callsite returns Interest::sometimes(). Interest::and collapses any mismatched pair to sometimes(), so every rebuild now yields at worst sometimes -> warn! always consults the current thread-local dispatcher -> scoped capture deterministically sees its own event. No-op sink (enabled=false) so no other test's captured set changes.

PLACEMENT: install_permissive_global_test_subscriber() in server.rs dispatch_tests (bin crate, where the flake lives). Called at top of the flaky test + the other two server dispatch_tests capture tests.

VERIFY: harness loops full bin suite (cargo test -p whereamid --bin whereamid). Unfixed 40x: caught target FAIL (detector validated). Fixed: require 0 target failures across >=300.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Session 2026-07-08: diagnosed + reproduced (1/60 then 12/100), attempted fixes (a-precursor) sometimes() and whole-scope with_default — BOTH failed, reverted to clean master (server.rs untouched on disk). Left In Progress work as durable plan above. Stopping per honest-failure discipline: two confident fixes both wrong = the overconfidence stop signal. This is a cross-cutting tracing-test-infra change, not the quick de-flake it first looked like — wants a fresh dedicated context. The gate flake REMAINS (~12% under load); treat `just e2e` as not-yet-reliable until this lands.

Session 2026-07-08 (fresh context, cycle 16 resumed):
- Read tracing-core 0.1.36 source directly to correct the prior mental model. KEY correction: thread-local dispatchers DO enter the DISPATCHERS registry (via Dispatch::new); the never-clobber comes from an empty/mismatched fold at first-registration, and Interest::and makes a single sometimes() sticky. This is why a permanent global sometimes() member is decisive.
- Implemented install_permissive_global_test_subscriber() (server.rs dispatch_tests) + wired into the 3 server capture tests. WARN assertion UNCHANGED (AC#3 preserved).
- Harness validated on UNFIXED (git stash) 40x: target test FAILED at run 14 -> detector proven to catch the flake; base rate reconfirmed present.
- INCIDENTAL: harness also caught a SECOND, unrelated pre-existing flake at run 19: scanner::tests::tokio_runner_kills_child_when_scan_future_is_cancelled (33s timeout under load) -> filed task-0249 (NOT in scope for 0248).
- Fixed 300x verification launched (decisive: require 0 target failures).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Root-caused and fixed the tracing-capture flake at the harness level, verified decisively under load.

ROOT CAUSE (confirmed against tracing-core 0.1.36 source): tracing keeps a process-global per-callsite Interest cache. rebuild_callsite_interest folds each registered dispatcher's register_callsite via Interest::and and falls back to Interest::never() on an empty fold. The handle_locate non-finite warn! callsite is exercised by only ONE scoped test; under parallel cargo test it could be first-registered in a window where that test's dispatcher is not folded in -> cached never -> warn! short-circuits -> event dropped -> capture sees []. The two prior in-code fixes failed because they only touched the thread-local capturing subscriber, never a permanent member of the dispatcher registry the fold reads.

FIX: install once (std::sync::Once) a process-global default subscriber whose register_callsite returns Interest::sometimes() and which is a no-op sink (enabled()=false). Interest::and collapses any mismatched pair to sometimes(), and set_global_default leaks the dispatch so its weak registrar is a permanent fold member -> no callsite can ever be cached never -> warn! always consults the current thread-local dispatcher -> scoped capture is deterministic. Algebraic invariant, not probabilistic. The exact-match WARN assertion is UNCHANGED (AC#3). Wired into the three server::dispatch_tests capture tests.

VERIFICATION: harness loops the full whereamid bin suite (drives the parallel churn). Detector validated on the UNFIXED tree (git stash): target FAILED at run 14/40. FIXED tree: 0 target failures across 300 runs; QA reviewer independently 0/60 (0/360 cumulative). clippy -D warnings clean; fmt clean; full workspace 736 tests pass. Review gate: qa-test-runner GO, mped-architect GO (conditional; conditions met — AC#2 green + docstring corrected). In-thread review hardening: corrected the misleading "no other test's captured set changes" comment, documented the set_global_default Err silent-no-op failure mode, dropped dead pub(crate) visibility.

SCOPE/HONEST LIMITS: this hardens the three server capture tests and decisively fixes the reported gate flake. The root cause is process-global; sibling capture tests (db.rs exact-count in the lib binary, config.rs absence tests, trilaterate.rs) share the same latent mechanism and are order-dependent across run shapes — filed as task-0251 (not observed flaking in 360+ runs, latent only). Two UNRELATED pre-existing gate flakes surfaced during verification: task-0249 (scanner child-kill timeout ~1.7%) and task-0250 (post_commit_wipe subprocess timing ~0.3%).
<!-- SECTION:FINAL_SUMMARY:END -->
