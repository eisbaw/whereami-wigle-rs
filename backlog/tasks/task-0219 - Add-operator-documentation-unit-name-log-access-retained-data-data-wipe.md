---
id: TASK-0219
title: 'Add operator documentation: unit name, log access, retained data, data-wipe'
status: Done
assignee: []
created_date: '2026-07-02 04:54'
updated_date: '2026-07-05 07:39'
labels:
  - docs
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The README enables the service but lacks a complete operator runbook: exact NixOS and Home Manager unit names and journal commands, a first-class service logging filter, deployment-specific database paths and permissions, current table-by-table retention semantics, and an instance-pinned verified wipe procedure. Current behavior after TASK-0202 is 30-day default pruning for both fixes and resolved AP cache rows, with 0 keeping either indefinitely; last_fix has no independent age window, while pending and not_found are lifecycle-managed. No man page or generated shell completions are shipped, so their support boundary must be explicit.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 docs state the systemd unit name and a one-line journalctl command, and how to set RUST_LOG via the module
- [x] #2 docs enumerate retained personal data (tables, default retention, path) and a verified wipe procedure
- [x] #3 man page/completions either shipped (clap_mangen/clap_complete) or explicitly declared out of scope
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Treat the task description as historical and source every operator claim from the current daemon, SQLite schema, Nix modules, and TASK-0202 wipe contract. 2. Add a shared nullable services.whereami.rustLog option and wire it safely into both NixOS and Home Manager systemd units; make the NixOS module install cfg.package so its documented client commands exist. 3. Expand README with separate NixOS/Home Manager operations, exact unit/log/status/recovery commands, temporary debug-log privacy guidance, credentials handling, deployment-specific paths and permissions, a table-by-table retained-data/lifetime matrix, socket-pinned wipe procedures, and an explicit no-man-page/no-completion decision. Correct adjacent stale TCP-first and unprivileged quick-start text only where necessary for an executable guide. 4. Ground the new contract with Nix/default assertions and VM coverage proving CLI availability, effective RUST_LOG, a deterministic Unix-peer debug event, exact socket-targeted wipe, and any exact restart values documented; reuse existing retention/wipe/permission/credential integration coverage. 5. Run focused checks and nix-shell just e2e, then require parallel qa-test-runner and mped-architect GO, complete the tracker, rerun the mandatory E2E gate, and commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Grounding carried from TASK-0197/0202/0204/0205/0206: the NixOS state directory is 0700 and aps.sqlite plus WAL/SHM are 0600; Home Manager and manual deployments create missing parent directories 0700 and tighten database files best-effort, while a pre-existing loose parent is warned rather than chmodded. The aps table contains resolved stable cached access points, not every visible BSSID/SSID; unresolved stable observations can enter pending, and transient scans remain managed memory. NixOS credentialsFile uses systemd LoadCredential from a 0600/0400 root-owned source; Home Manager/manual read a user-owned file. History and AP retention each default to 30 days and 0 disables pruning. The system unit uses Restart=on-failure with a bounded 5-start/60-second limit. Preimplementation contract (2026-07-05): add shared nullable rustLog for both module variants, make the NixOS module install its client, document separate NixOS/Home Manager operations, pin wipe commands to the intended Unix socket, enumerate every privacy-bearing table and actual lifetime, distinguish logical wipe from forensic erasure, state that logs are outside wipe, and explicitly leave man pages/generated completions out of scope. Final-review NO-GO (2026-07-05): the supported LoadCredential copy is presented as protected mode 0440 but the generic credentials check emits a false other-user warning; recognize only the exact CREDENTIALS_DIRECTORY/config delivery, retain warnings for arbitrary 0440 files, remove the incorrect 0400-copy claim, and assert absence in the VM. Correct logging text because normal warnings may include BSSIDs while debug adds coordinates and peer identity. Say rustLog=null omits only the unit-specific assignment and falls back to info only without inherited RUST_LOG. Call aps resolved/stable, retain allowedUsers plus root destructive-wipe VM coverage, specify overridden packages must contain both binaries, and pin both CR and LF rejection. TASK-0218 remains responsible for stale beacondb option text.

Second final-review NO-GO (2026-07-05): the exact CREDENTIALS_DIRECTORY/config lexical path can be a symlink to an ordinary 0440 target and still receive the exemption. Require the path entry itself to be a non-symlink that still identifies the opened descriptor, preserve descriptor-based metadata/read, and add exact-name symlink coverage. Add exact managed-path 0444 and 0640 tests so future/unsafe modes fail closed with a warning. Correct README permission scope: only directly-read Home Manager/manual files receive daemon mode warnings; the NixOS source is read by systemd and must be secured independently, while only its protected managed copy is inspected/exempted by the daemon.

Final remediation verification (2026-07-05): the systemd credential exemption additionally requires a direct regular non-symlink path entry whose device/inode matches the already-opened descriptor. Symlink, replacement, missing/inspection failure, 0444/0640, ordinary 0440, and path-lookalike regressions all retain warnings. The real NixOS VM credential remains accepted without a false warning. Fresh QA and architecture reviews on frozen hash f817e2c1 returned GO with no findings.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added a code-grounded operator handbook covering NixOS and Home Manager unit names, journal/status/restart recovery, safe credentials, deployment-specific database paths and permissions, every retained table and actual lifetime, socket-pinned logical wipe procedures, and the explicit decision that man pages/generated completions are not shipped. Added shared nullable services.whereami.rustLog wiring with systemd-safe escaping, single-line validation, option/default contracts, and honest inherited-environment/privacy guidance. The NixOS module now installs the client as well as running the daemon, and custom packages must provide both binaries. Corrected a pre-existing false credential warning by narrowly recognizing only the exact protected systemd 0440 direct file that matches the opened descriptor; arbitrary, unsafe, replaced, missing, and symlink paths still warn. VM coverage proves CLI availability, logging, restart policy, secure credentials, allowedUsers and root socket-pinned wipes. Verified 474-test E2E, formatting, 25-flag parity, option contracts, all 17 VM subtests, and final parallel QA/architecture GO with no findings.
<!-- SECTION:FINAL_SUMMARY:END -->
