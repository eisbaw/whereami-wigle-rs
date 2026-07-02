---
id: TASK-0209
title: Add nix flake checks + a NixOS VM test for the module
status: To Do
assignee: []
created_date: '2026-07-02 04:53'
updated_date: '2026-07-02 23:06'
labels:
  - nix
  - testing
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
flake.nix has no checks output and package.nix has no passthru.tests, so nix flake check only evaluates - runs no tests, no clippy, never boots the module. There is no nixosTest covering the unit. Nothing automated would catch the world-readable DB, the DynamicUser/secrets crash, or the negative-int option crash - all boot-time behaviors a VM test surfaces immediately. Ref flake.nix:18-78, nix/package.nix:57-61.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 nix flake check fails on a clippy error or failing unit test
- [ ] #2 a NixOS VM test boots nixosModules.default, enables the service, and asserts it reaches active and answers on 127.0.0.1:4747
- [ ] #3 the VM test asserts DB/state-dir permissions are not world-readable
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from task-0197 (state dir/DB perms): the VM/flake test MUST assert the resulting mode bits, which task-0197 could only guarantee declaratively (StateDirectoryMode/UMask) + via the daemon backstop, NOT runtime-verify locally. Specifically assert:
- system unit: /var/lib/whereami (following the DynamicUser /var/lib/private symlink) is 0700, and aps.sqlite is 0600 (WAL sidecars -wal/-shm also 0600).
- hm unit: %h/.local/share/whereami is 0700 and aps.sqlite is 0600.
This closes task-0197 AC#1s disclosed nix-path verification gap. Also a good place to assert the real-binary COMPOSITION (main() ensure_private_dir before Database::open), which 0197 verified only at unit level.
<!-- SECTION:NOTES:END -->
