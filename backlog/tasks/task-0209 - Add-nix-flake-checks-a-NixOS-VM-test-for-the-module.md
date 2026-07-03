---
id: TASK-0209
title: Add nix flake checks + a NixOS VM test for the module
status: To Do
assignee: []
created_date: '2026-07-02 04:53'
updated_date: '2026-07-03 05:50'
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

forward-carried from task-0196: the NixOS VM test must additionally assert:
(a) /run/whereami/whereamid.sock exists with mode 0660 root:whereami (created by the whereamid.socket unit BEFORE the DynamicUser service starts).
(b) a user NOT in services.whereami.allowedUsers gets connection refused / EACCES connecting to the socket.
(c) a user IN allowedUsers gets a valid version response.
(d) hm variant: $XDG_RUNTIME_DIR/whereamid.sock is mode 0600 and the owner query works.
(e) with cfg.bind set non-loopback and allowNonlocalBind=false the daemon refuses to start (daemon-side validate() rejects it; there is NO nix-side IP parse, so config eval succeeds and the daemon exits nonzero).
GOTCHA to verify: module.nix relies on systemd auto-creating the ListenStream parent dir /run/whereami (mkdir_parents on AF_UNIX bind). Assert /run/whereami exists; if systemd does NOT create it, add a tmpfiles rule / RuntimeDirectory.
Also assert the socket-activated service logs "adopting systemd socket-activated fd 3" and does NOT self-bind.
<!-- SECTION:NOTES:END -->
