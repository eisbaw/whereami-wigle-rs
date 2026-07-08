---
id: TASK-0248
title: >-
  De-flake task-0239 tracing-capture test
  (locate_skips_non_finite_cached_ap_and_warns_with_bssid) — ~1-in-8 under
  parallel load
status: To Do
assignee:
  - '@myself'
created_date: '2026-07-07 23:24'
updated_date: '2026-07-08 06:37'
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
- [ ] #1 Root cause of the empty-capture flake is identified and fixed at the harness level (not by weakening the WARN assertion)
- [ ] #2 The test passes >=50/50 under full-parallel just e2e load (measured, not a single sample)
- [ ] #3 The non-finite-skip WARN behavior is still positively asserted (the guard still bites)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
CONFIRMED root cause (session 2026-07-08 diagnosis): tracing keeps a PROCESS-GLOBAL per-callsite Interest cache. Under cargo test's parallel harness, any scoped-subscriber test entering/leaving `with_default`/`with_subscriber` triggers a global rebuild_interest_cache that can clobber the WARN callsite (server.rs:1902) to `never` in a window where our capturing subscriber is not the one the rebuild sees — so the `warn!` short-circuits BEFORE consulting the active dispatcher and the event is dropped. There are 9+ scoped-subscriber sites across FOUR modules (server.rs x4, db.rs x3, trilaterate.rs, config.rs) all churning this global state; that concurrency is the real driver (measured ~12% under full bin-suite parallelism).

TWO FIXES ATTEMPTED THIS SESSION — BOTH FAILED (reverted):
1. register_callsite -> Interest::sometimes() on CapturedEvents (defeat always/never caching) + keep per-poll WithSubscriber: still flaked 1/60.
2. #1 plus rewrite the test to whole-scope tracing::subscriber::with_default over a manual current-thread runtime (dispatcher active for the entire block_on, mirroring the reliable synchronous with_default tests): FLAKED 12/100. So neither sometimes() nor holding our dispatcher for the whole test wins the cross-thread global-cache race.

CANDIDATE FIXES for the next session (ranked):
(a) Serialize ALL scoped-subscriber tests behind one process-global Mutex so no two dispatcher-changing tests run concurrently (removes the parallel rebuild that clobbers the cache). Requires a shared guard used at all 9+ sites across the 4 modules (today each module has its own CapturedEvents copy) — cross-cutting but conceptually simple. Verify it actually stops the clobber (thread-local dispatchers may or may not participate in rebuilds).
(b) Install ONCE (std::sync::Once) a process-global default subscriber whose register_callsite returns Interest::sometimes() and whose event() is a no-op sink, so every callsite is permanently `sometimes` -> per-event enabled() is always evaluated against the CURRENT thread dispatcher -> capture tests deterministically see their own events. One localized change but changes global test tracing behavior; verify it does not perturb the other 8 capture tests.
(c) Abandon tracing-capture for THIS assertion: extract the non-finite-skip+warn loop into a small unit and assert the diagnostic deterministically without the async/global-dispatcher path. Most robust, but a production refactor of handle_locate.

VERIFICATION BAR: base rate is ~12% under full bin-suite parallelism (repro harness: loop `cargo test -p whereamid --bin whereamid` >=100x and grep for the test). A fix must show 0 failures across >=300 runs to be decisive (0 across 100 is NOT enough). Do NOT weaken the WARN assertion to hide the drop (AC#3).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Session 2026-07-08: diagnosed + reproduced (1/60 then 12/100), attempted fixes (a-precursor) sometimes() and whole-scope with_default — BOTH failed, reverted to clean master (server.rs untouched on disk). Left In Progress work as durable plan above. Stopping per honest-failure discipline: two confident fixes both wrong = the overconfidence stop signal. This is a cross-cutting tracing-test-infra change, not the quick de-flake it first looked like — wants a fresh dedicated context. The gate flake REMAINS (~12% under load); treat `just e2e` as not-yet-reliable until this lands.
<!-- SECTION:NOTES:END -->
