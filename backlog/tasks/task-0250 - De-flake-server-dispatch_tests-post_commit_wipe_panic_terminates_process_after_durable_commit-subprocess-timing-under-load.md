---
id: TASK-0250
title: >-
  De-flake
  server::dispatch_tests::post_commit_wipe_panic_terminates_process_after_durable_commit
  (subprocess timing under load)
status: To Do
assignee:
  - '@myself'
created_date: '2026-07-08 08:23'
updated_date: '2026-07-08 22:19'
labels:
  - robustness
  - test-flake
  - tech-debt
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
A THIRD pre-existing e2e flake (distinct from task-0248 tracing-capture and task-0249 scanner-kill), surfaced by the task-0248 verification harness: 1/300 full-parallel bin-suite runs on the FIXED tree failed with

  test server::dispatch_tests::post_commit_wipe_panic_terminates_process_after_durable_commit ... FAILED

The test spawns a real child daemon process, drives it to a post-commit panic, and asserts the process self-terminates after the durable commit (uses wait_for_child_marker with a fixed budget). Under heavy parallel cargo-test load the child spawn/marker/terminate timing can exceed the budget -> intermittent failure (~0.3% observed). Product behavior (panic-after-commit self-termination) is correct and separately covered; this is a TEST timing defect. Fix direction: make the child readiness/termination waits robust to scheduler starvation (widen or poll-with-generous-deadline, event-driven where possible). Verify with an N-times (>=300) parallel-load loop given the low base rate. Do NOT weaken the termination assertion.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Triaged 2026-07-09 (NOT fixed — needs a dedicated session with a multi-hour verification budget). The test (server.rs:5610, #[test] post_commit_wipe_panic_terminates_process_after_durable_commit) spawns a NESTED test-binary child (WIPE_FAIL_STOP_CHILD_TEST) and waits on filesystem markers + child exit with 10s wall-clock budgets (already blocking-std + Instant, like task-0249). The ~0.3% flake is a subprocess-TIMING miss under full-parallel `just e2e` load: the nested child, contending with the 14-core outer suite, occasionally does not reach a marker (fatal_entered / reader_waiting) or exit within 10s. UNLIKE task-0249 (where I decoupled the assertion from tokio's async reaper entirely), here the assertion inherently must WAIT for a subprocess to reach a state — there is no clean mechanistic decoupling; the only lever is a more generous budget (widen 10s -> e.g. 60s), which is a MITIGATION not an elimination. PROCESS for a fresh session: (1) REPRODUCE FIRST — run the full bin suite in a loop (cargo test -p whereamid --bin whereamid, ~300-600x, capturing FULL output) to catch the ~0.3% flake and see WHICH assertion fails (fatal_entered? reader_waiting? timed_out? status.code!=70?) — do NOT assume it is the budget without confirming; it could be a subtler race (e.g. child killed at deadline -> status None -> code!=70). (2) Fix per the actual failure (likely widen the three 10s budgets, or make the child less contended). (3) VERIFY with a large-N loop sized to the ~0.3% base (>=500-1000 runs, 0 failures) — expensive (~2-3hr background at ~11s/run). This is the LAST known gate flake (0248+0249 fixed); fixing it makes `just e2e` fully reliable. Cross-ref task-0249 (the sibling child-timing de-flake + the flake_harness.sh pattern).
<!-- SECTION:NOTES:END -->
