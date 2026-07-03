---
id: TASK-0207
title: NixOS module integer options accept negatives/zero -> runtime crash
status: To Do
assignee: []
created_date: '2026-07-02 04:53'
updated_date: '2026-07-03 07:12'
labels:
  - nix
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
All numeric options in options.nix use lib.types.int, which accepts negatives and zero. The daemon rejects these at runtime in Args::validate (e.g. --pending-max-attempts must be >0, --top-n must be >0), so services.whereami.topN=0 type-checks, builds, and then crash-loops on boot - the worst time to learn. Interacts with the missing Restart to hot-loop. Ref nix/options.nix:44-102, whereamid/src/config.rs:112-162.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 topN=0 or any <=0 count fails nixos-rebuild evaluation, not at runtime (ints.positive/unsigned/between)
- [ ] #2 option numeric bounds mirror the daemon's validate() invariants
- [ ] #3 valid configurations continue to evaluate unchanged
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carry from task-0209: the NixOS VM test (nix/vm-test.nix) is the natural place to prove this. The nonloopback node already demonstrates the pattern — boot the module with a bad option and assert the unit fails (poll `systemctl is-failed`, since a failed unit hangs wait_for_unit). options.nix uses lib.types.int with no lower bound; the daemon's clap value_parser + Args::validate() is the current runtime guard. A VM assertion (boot with e.g. debounceWindow=0 or a negative int, expect the unit to fail naming the offending flag) would lock the guard in. Same runNixOSTest gotchas apply: gate journal reads on the daemon `... unix socket ready` line; do not name a node after a Python keyword.
<!-- SECTION:NOTES:END -->
