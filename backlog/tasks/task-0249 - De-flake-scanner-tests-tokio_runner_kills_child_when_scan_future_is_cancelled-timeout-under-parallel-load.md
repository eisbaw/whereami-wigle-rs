---
id: TASK-0249
title: >-
  De-flake
  scanner::tests::tokio_runner_kills_child_when_scan_future_is_cancelled
  (timeout under parallel load)
status: Done
assignee:
  - '@myself'
created_date: '2026-07-08 07:38'
updated_date: '2026-07-08 13:43'
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

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Root cause of the flake identified and fixed at the test-harness level (not by weakening the kill assertion)
- [x] #2 Target test passes >=300/300 under full-parallel bin-suite load (measured, not a single sample)
- [x] #3 The cancel-kills-child behavior is still positively asserted (the child must be killed)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
ROOT CAUSE (verified against tokio 1.52.1 source):
- kill_on_drop(true): when the aborted task future (and its tokio Child) is dropped, ChildDropGuard::drop calls Kill::kill -> SIGKILL delivered SYNCHRONOUSLY (mod.rs:1124). The kernel turns the child into a ZOMBIE near-instantly.
- /proc/<pid> persists for a zombie until tokio's BEST-EFFORT async reaper (OrphanQueue / pidfd reaper) runs waitpid (docs mod.rs:211-218).
- The OLD test polled /proc/<pid> EXISTENCE via a tokio timeout on a current-thread runtime. Under parallel cargo-test load that runtime (hence the reaper) is starved, so the un-reaped zombie lingers >5s -> a correctly-killed child is misread as alive. The test conflated "zombie awaiting reap" with "still running", and its own timer depended on the starved runtime.

FIX: assert the child is DEAD (gone OR zombie) rather than fully reaped, and poll with BLOCKING std + a wall-clock deadline (20s) so the verdict is independent of the async runtime. child_is_dead(pid) reads /proc/<pid>/stat: missing => reaped/dead; state Z => zombie/dead; else alive. SIGKILL is uncatchable + synchronous at drop, so the child is ALWAYS dead/zombie -> mechanistically the new check cannot false-fail; kernel teardown is not load-sensitive so the 20s budget is only consumed on a genuine still-running regression. Assertion NOT weakened (still requires the child be killed). Safety SIGKILL+panic kept for a genuine leak.

VERIFY: full-bin-suite harness (parallel load required). Unfixed 80x: target FAILED 1/80 (detector validated). Fixed: require 0 target failures across >=300.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Session 2026-07-08 (cycle 17):
- Confirmed root cause of the PRIMARY flake against tokio 1.52.1 source: kill_on_drop kills synchronously at drop, but /proc/<pid> lingers for the zombie until tokio's best-effort async reap, which is starved under parallel load. Fixed phase 2: assert dead-or-zombie via blocking /proc/<pid>/stat state read + wall-clock budget (runtime-independent).
- OVERCONFIDENCE MISS (honest disclosure): I declared the test fixed on a mechanistic argument, but the 300x verification still showed 1/300 target failures at scanner.rs:2004:26 — a SECOND, independent race in PHASE 1: the child writes its PID via `printf $$ > file`; the shell `>` creates/truncates the file EMPTY before printf writes, so a racing read sees empty content and .parse::<u32>().expect() panicked. This was masked by the more-frequent reap flake. Loop lesson reinforced (again): do NOT declare a flake fixed on reasoning alone; the large-N loop is what caught my incomplete fix.
- Phase-1 fix: treat "exists but not yet a valid PID" as not-ready and keep polling (no expect-panic); widened budget 5s->10s. Kept async sleep (phase-1 shares the single current-thread runtime with the child-spawn task, so it must yield).
- Re-verifying with 400x (both races addressed).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
De-flaked scanner::tests::tokio_runner_kills_child_when_scan_future_is_cancelled, root-caused against tokio 1.52.1 source, verified 0/400 under load.

TWO independent races under parallel cargo-test load:
1. PRIMARY (~1.3%): kill_on_drop delivers SIGKILL synchronously at Child drop, but /proc/<pid> lingers as a ZOMBIE until tokio's best-effort async reaper runs — starved under load. The old check polled /proc/<pid> EXISTENCE via a tokio timeout on the (starved) current-thread runtime, misreading a killed-but-unreaped zombie as alive. Fix: child_is_dead() treats gone OR /proc/<pid>/stat state Z/X/x as dead, polled with BLOCKING std + a 20s wall-clock deadline (runtime-independent). The loop is deliberately yield-free so the reaper cannot free/reuse the pid mid-check (protects the safety-net libc::kill).
2. SECONDARY (~0.3%, exposed only after fixing #1 — the 300x loop caught 1/300 at scanner.rs:2004): the shell `>` truncates the pidfile empty before printf writes, so a racing read saw empty content and .parse().expect() panicked. Fix: poll until a valid pid appears (no panic); budget 5s->10s.

Assertion NOT weakened (still requires the child be killed). Documented the current-thread flavor dependency and the intentional async(phase-1)/blocking(phase-2) sleep asymmetry to prevent a future cleanup silently reintroducing either race.

VERIFICATION: detector validated on UNFIXED tree = target FAILED 1/80; FIXED tree = 0 target failures / 400 runs (+ QA 0/40). clippy -D warnings + fmt clean; 734 workspace tests pass. Review qa GO / mped GO (design GO; conditions met + P1/P3/P4 hardening applied in-thread: yield-free-loop invariant, Z/X/x widening, flavor-dependency header comment).

HONEST NOTE: I first declared phase-2 alone complete on a mechanistic argument; the large-N loop exposed phase 1. Lesson reinforced — never certify a flake fix on reasoning alone; the base-rate-sized loop is the proof.
<!-- SECTION:FINAL_SUMMARY:END -->
