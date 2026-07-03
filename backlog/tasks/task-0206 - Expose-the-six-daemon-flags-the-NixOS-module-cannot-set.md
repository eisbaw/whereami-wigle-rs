---
id: TASK-0206
title: Expose the six daemon flags the NixOS module cannot set
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
options.nix (17 options) + exec-start.nix (13 flags) vs config.rs (21 flags): --address-cache-ttl-days, --http-timeout-secs, --nominatim-timeout-secs, --history-retention-days, --history-segment-distance-m, --history-segment-min-duration-secs have no module option and are never emitted. Most significant: --history-retention-days governs how long location PII is kept, silently pinned to the 30-day compiled default with no module knob. Ref nix/options.nix, exec-start.nix:8-24, whereamid/src/config.rs:11-102.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 every whereamid CLI flag has a corresponding services.whereami.* option emitted by exec-start.nix
- [ ] #2 services.whereami.historyRetentionDays measurably changes the daemon's --history-retention-days
- [ ] #3 a parity check (test or comment) guards against future flag/option drift
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
forward-carried from task-0196: options.nix gained socketPath, bind (now nullOr, default null), allowNonlocalBind, and allowedUsers; exec-start.nix now conditionally emits --socket/--bind/--allow-nonlocal-bind via a socketFlag param (fixed list + optionalString appends). Re-derive the missing-flag list against the CURRENT exec-start.nix before implementing — the structure changed and some targeted flags may now be covered.

Forward-carry from task-0209: nix/vm-test.nix now exists (pkgs.testers.runNixOSTest, 2 nodes: machine + nonloopback). Extend THAT test here rather than writing a new one. Gotchas learned:
- Node names become Python variables in the test script; never name a node after a Python keyword (`nonlocal` broke the mypy type-check AND would break at runtime).
- Type=simple socket-activated services report `active` at exec, before the daemon finishes DB migrations / reaches its listener. Gate journal assertions on the daemon's own `control channel: unix socket ready` line (wait_until_succeeds), not just wait_for_unit.
- A failed unit hangs wait_for_unit; poll `systemctl is-failed --quiet` instead (see the nonloopback node).
<!-- SECTION:NOTES:END -->
