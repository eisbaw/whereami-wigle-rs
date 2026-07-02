---
id: TASK-0218
title: 'Remove dead [beacondb] block from nix options.nix credentialsFile docs'
status: To Do
assignee: []
created_date: '2026-07-02 04:54'
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
- [ ] #1 options.nix documents only keys ConfigFile actually parses
- [ ] #2 module docs render without the dead [beacondb] block
- [ ] #3 optionally the daemon warns on unknown top-level tables in the config TOML
<!-- AC:END -->
