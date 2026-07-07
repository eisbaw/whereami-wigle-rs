---
id: TASK-0248
title: >-
  De-flake task-0239 tracing-capture test
  (locate_skips_non_finite_cached_ap_and_warns_with_bssid) — ~1-in-8 under
  parallel load
status: To Do
assignee: []
created_date: '2026-07-07 23:24'
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
