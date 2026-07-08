---
id: TASK-0250
title: >-
  De-flake
  server::dispatch_tests::post_commit_wipe_panic_terminates_process_after_durable_commit
  (subprocess timing under load)
status: To Do
assignee: []
created_date: '2026-07-08 08:23'
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
