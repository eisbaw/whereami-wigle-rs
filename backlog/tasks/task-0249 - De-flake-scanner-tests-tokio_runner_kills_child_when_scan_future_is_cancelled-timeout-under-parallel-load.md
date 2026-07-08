---
id: TASK-0249
title: >-
  De-flake
  scanner::tests::tokio_runner_kills_child_when_scan_future_is_cancelled
  (timeout under parallel load)
status: To Do
assignee: []
created_date: '2026-07-08 07:38'
labels:
  - robustness
  - test-flake
  - tech-debt
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
A SECOND pre-existing e2e flake, distinct from task-0248, surfaced by the task-0248 verification harness (running the full whereamid bin suite in a loop). On the unfixed baseline, run 19/40 failed with:

  test scanner::tests::tokio_runner_kills_child_when_scan_future_is_cancelled ... FAILED
  test result: FAILED ... finished in 33.44s

The 33.44s suite time (vs ~5-11s normal) shows this is a TIMING/timeout flake: under full-parallel cargo-test load the child-kill / cancellation path exceeds its wait budget rather than the tracing-capture mechanism of task-0248. This is a genuine gate-reliability defect (just e2e can still redden on it). Fix direction: identify the timing assumption in the test (child spawn + cancel + kill_on_drop reap) and make the wait budget robust to scheduler starvation, OR make the readiness/kill observation event-driven rather than fixed-duration. Verify with an N-times (>=100) parallel-load loop sized to the base rate. Do NOT weaken the cancellation assertion.
<!-- SECTION:DESCRIPTION:END -->
