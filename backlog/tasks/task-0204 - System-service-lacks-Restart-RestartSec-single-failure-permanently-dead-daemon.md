---
id: TASK-0204
title: >-
  System service lacks Restart=/RestartSec= (single failure = permanently dead
  daemon)
status: To Do
assignee: []
created_date: '2026-07-02 04:53'
updated_date: '2026-07-02 22:23'
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
- [ ] #1 SIGKILL of the daemon results in an automatic restart within a few seconds
- [ ] #2 a non-zero exit puts the unit through auto-restart, not terminal failed
- [ ] #3 restart backoff is bounded (StartLimitIntervalSec/Burst) so a hard-failing binary does not hot-loop
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from task-0195: DECISION POINT for systemd Restart=. The daemon now exits 0 (clean Ok(())) on a bind failure — the server task logs the error and main runs the normal shutdown drain. With Restart=on-failure that clean exit-0 would NOT trigger a restart, so a transient port conflict leaves the daemon permanently dead. Either use Restart=always, OR change main to propagate a non-zero exit when the server task ends on its own (bind failure) rather than via a shutdown signal. The `server_exited` flag in main.rs already distinguishes that case cleanly.
<!-- SECTION:NOTES:END -->
