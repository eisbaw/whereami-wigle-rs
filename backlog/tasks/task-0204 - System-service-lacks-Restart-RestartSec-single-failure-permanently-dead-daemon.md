---
id: TASK-0204
title: >-
  System service lacks Restart=/RestartSec= (single failure = permanently dead
  daemon)
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-07-02 04:53'
updated_date: '2026-07-03 09:59'
labels:
  - nix
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
nix/module.nix serviceConfig omits Restart, so systemd default Restart=no applies. The daemon ?-propagates fatal startup errors (load_config_file?, Database::open?) and can panic mid-run; any such exit leaves the unit failed until manual restart or reboot. The hm-module already sets Restart=on-failure/RestartSec=5 - the production unit is worse than the user one. Ref nix/module.nix:20-39 vs hm-module.nix:25-26.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 SIGKILL of the daemon results in an automatic restart within a few seconds
- [x] #2 a non-zero exit puts the unit through auto-restart, not terminal failed
- [x] #3 restart backoff is bounded (StartLimitIntervalSec/Burst) so a hard-failing binary does not hot-loop
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Batched with 0205-0208 (one nix surface: module.nix/options.nix/exec-start.nix/hm-module.nix/vm-test.nix/flake.nix). This plan holds the shared VM strategy.

0204 change (system unit, module.nix serviceConfig + unit-level):
- Restart="on-failure"; RestartSec=2; startLimitIntervalSec=60; startLimitBurst=5 (StartLimit* are UNIT-level top attrs in NixOS, NOT serviceConfig).
- Rationale: matches hm-module (task frames prod as worse than user). on-failure covers SIGKILL (signal death), panic (exit 101), and validate()/DB-open non-zero (?-propagated). RestartSec=2 keeps the nonloopback start-limit fast+deterministic (5x2=10s << interval 60s -> burst exhausts -> failed).
- RESIDUAL (from 0195 forward-carry): a runtime TCP-bind failure (only when bind!=null, opt-in) makes the server task self-exit while main returns Ok(0); on-failure will NOT restart that. 0195 DELIBERATELY made that path exit-0 and pinned it (tests/shutdown.rs bind_failure_exits_cleanly_without_panic asserts status.success()). Changing it would break a Done task. Not in scope for 0204; will file a follow-up. Default system config (socket-activated unix, bind=null) has no such path, so on-failure fully protects it.

SHARED VM strategy (nix/vm-test.nix, the 0209 vehicle; extend, do not replace):
- machine node = kitchen-sink canonical deployment; add new subtests there.
- 0204: record MainPID, systemctl kill -s SIGKILL, wait until MainPID changes AND !=0, then alice version --json returns ok:true (restart that leaves a broken socket = fail). Placed LAST on machine.
- nonloopback: keep is-failed assert; verify it still reaches terminal failed under the restart loop.
- Bite: without Restart, SIGKILL leaves MainPID=0 -> the wait times out (load-bearing).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from task-0195: DECISION POINT for systemd Restart=. The daemon now exits 0 (clean Ok(())) on a bind failure — the server task logs the error and main runs the normal shutdown drain. With Restart=on-failure that clean exit-0 would NOT trigger a restart, so a transient port conflict leaves the daemon permanently dead. Either use Restart=always, OR change main to propagate a non-zero exit when the server task ends on its own (bind failure) rather than via a shutdown signal. The `server_exited` flag in main.rs already distinguishes that case cleanly.

Forward-carry from task-0209: nix/vm-test.nix now exists (pkgs.testers.runNixOSTest, 2 nodes: machine + nonloopback). Extend THAT test here rather than writing a new one. Gotchas learned:
- Node names become Python variables in the test script; never name a node after a Python keyword (`nonlocal` broke the mypy type-check AND would break at runtime).
- Type=simple socket-activated services report `active` at exec, before the daemon finishes DB migrations / reaches its listener. Gate journal assertions on the daemon's own `control channel: unix socket ready` line (wait_until_succeeds), not just wait_for_unit.
- A failed unit hangs wait_for_unit; poll `systemctl is-failed --quiet` instead (see the nonloopback node).

Implemented (commit f6bb2da, fix(nix)). module.nix: Restart="on-failure", RestartSec=2 (serviceConfig); startLimitIntervalSec=60, startLimitBurst=5 (UNIT-level top attrs — NOT serviceConfig, where they silently no-op). Chose on-failure (not always) to match the hm unit and the ACs; RestartSec=2 keeps recovery fast and the start-limit deterministic.

VM (machine node, new subtest): record MainPID, systemctl kill --signal=SIGKILL, wait MainPID becomes nonzero AND != old, wait_for_unit, alice version --json => ok:true.
- PASS observed: killed PID 1154 -> new PID in 2.52s -> answered (AC#1/#2).
- AC#3 bound proven by the nonloopback node under the new restart loop: 5 retries then journal "Start request repeated too quickly" / "Failed with result start-limit-hit" -> terminal failed; is-failed assert still passes (~8s).
- BITE watched: temporarily set Restart="no" + wait timeout=25 -> subtest FAILED "action timed out after 25.33s" (MainPID stayed 0). Reverted both.

Gate: nix flake check exit=0 (6 checks). Rust untouched.

RESIDUAL (from 0195 forward-carry): on-failure does NOT retry a clean exit(0); a runtime TCP-bind self-exit is exit(0) by deliberate, test-pinned 0195 design, so the opt-in bind case is not auto-restarted. Default system config (bind=null, socket-activated) has no such path. Filed follow-up (see below).

Review gate (cycle 10, batch 0204-0208, range 596f068..b50ab0e): qa-test-runner GO + mped-architect GO (mped re-run after an API-error death; QA evidence logs in /tmp/qa-*.log). QA: nix flake check exit 0 x2 (24s/12s cached), all 11 VM subtests executed and non-vacuous (SIGKILL subtest killed PID 1156 -> new PID 1387 re-adopted fd 3 and answered alice; LoadCredential pinned affirmative log + absent not-found warning + source still 0600 root:root; nonloopback traversed restart counters to start-limit-hit then is-failed), flag-parity "23 flags OK", Rust diff confirmed log-only, host end-state pristine. mped verified: startLimit* correctly UNIT-level; option types mirror validate() EXACTLY (no invented bounds; scanFastDuration/dailyLimit correctly unsigned — dailyLimit=0 proven to disable WiGLE via db.rs reserve logic); parity guard bites both directions and cannot vacuously pass; assertions.nix shared by both modules.

In-thread hardening 48647e0: module.nix residual comment redirected to task-0233 graceful-degrade direction (mped M-1: the 0237 filed this cycle was a duplicate of 0233 with a review-REJECTED fail-fast direction — archived, id later recycled by the CLI for the new sigterm task); vm-test SIGKILL settle-wait comment de-overclaimed (QA M1); flag-parity duplication comment (L-2). Filed (new) 0237: sigterm test de-flake — QA H1, pre-existing 300ms-sleep race, 1/4 failure under ambient load, intermittently blocks just e2e; ALSO flags a real early-SIGTERM window in the daemon. Accepted: L-1 u32/i32 upper-bound eval gap (disclosed in options.nix), L-4 start-limit transitively asserted, L-5 $CREDENTIALS_DIRECTORY env-expansion form (VM-proven).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Gave the system service auto-restart with bounded backoff, so a single fatal error no longer leaves the daemon permanently dead.

Changes (module.nix):
- serviceConfig: Restart="on-failure", RestartSec=2.
- Unit level (NOT serviceConfig, where they silently no-op): startLimitIntervalSec=60, startLimitBurst=5 — more than 5 failed starts in 60s and systemd stops retrying (no hot-loop).

on-failure was chosen over always to match the hm unit and satisfy the ACs (SIGKILL/signal death, panics, DB-open failures all restart).

Residual (documented, task-0195/0237): on-failure does not retry a clean exit(0); a runtime TCP-bind self-exit is exit(0) by deliberate, test-pinned design, so the opt-in bind case is not auto-restarted. The default config (bind=null, socket-activated) has no such path. Filed follow-up task-0237 to make server self-exit propagate non-zero.

Tests: nix flake check green. New VM subtest SIGKILLs the daemon -> it returns with a new MainPID (2.5s) and answers alice again. The nonloopback node now exercises the bound (5 retries -> start-limit-hit -> terminal failed; its is-failed assert still passes). Bite: Restart="no" -> the SIGKILL subtest times out (MainPID stays 0).
<!-- SECTION:FINAL_SUMMARY:END -->
