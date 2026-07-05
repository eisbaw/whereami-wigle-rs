---
id: TASK-0218
title: 'Remove dead [beacondb] block from nix options.nix credentialsFile docs'
status: Done
assignee: []
created_date: '2026-07-02 04:54'
updated_date: '2026-07-05 07:51'
labels:
  - docs
  - nix
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The credentialsFile option description instructs an [beacondb] enabled=true block, but ConfigFile deserializes only wigle:WigleConfig and the BeaconDB client was removed in 0.4.0. NixOS users copy the documented snippet believing they enabled a third backend; the unknown TOML table is silently ignored. Ref nix/options.nix:33-42 vs whereamid/src/config.rs:166-179.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 options.nix documents only keys ConfigFile actually parses
- [x] #2 module docs render without the dead [beacondb] block
- [ ] #3 optionally the daemon warns on unknown top-level tables in the config TOML
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Remove the dead [beacondb] example from the shared credentialsFile option so both NixOS and Home Manager generated documentation describe only the parsed [wigle] keys. 2. Extend the existing option-contract check to evaluate the real rendered description and assert that [wigle] remains while beacondb does not. Preserve legitimate legacy Source::BeaconDb database/protocol references. 3. Run the focused Nix contract and nix-shell just e2e, require parallel qa-test-runner and mped-architect GO, complete the tracker, rerun E2E, and commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Preimplementation contract (2026-07-05): ConfigFile currently parses only the wigle table. AC3 is explicitly optional; do not add deny_unknown_fields or change startup compatibility merely to remove stale generated documentation. The historical beacondb source enum and migration/protocol vocabulary remain valid for old database rows and are out of scope. Ground AC2 by evaluating the shared option description used by both module variants.

Final review verification (2026-07-05): evaluated NixOS and Home Manager descriptions are identical, retain the supported WiGLE keys, and contain no BeaconDB guidance. Parallel QA and architecture returned GO with no findings on frozen hash 3e82505b. Optional AC3 remains intentionally unimplemented and unchecked to preserve parser compatibility.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Removed the dead [beacondb] example from the shared credentialsFile option used by both NixOS and Home Manager. Added a semantic option-contract check proving the evaluated descriptions are identical, retain [wigle]/api_user/api_key, and contain no case-insensitive BeaconDB guidance. Preserved ConfigFile compatibility and all historical BeaconDB database/migration/protocol vocabulary. Verified the focused Nix check, 474-test E2E with strict Clippy, and final parallel QA/architecture GO with no findings. Optional unknown-table warnings were deliberately not implemented.
<!-- SECTION:FINAL_SUMMARY:END -->
