---
id: TASK-0168
title: Add cargo-audit + cargo-deny supply-chain hygiene
status: To Do
assignee: []
created_date: '2026-05-20 21:34'
labels:
  - tooling
  - security
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Forge F11. No cargo-audit, cargo-deny, or cargo-vet. For a daemon talking to WiGLE/Apple/Nominatim/BeaconDB over HTTPS, supply-chain hygiene is not optional.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Add pkgs.cargo-audit and pkgs.cargo-deny to the devShell
- [ ] #2 Commit a minimal deny.toml at workspace root (licenses allowlist, sources allowlist=crates-io, advisories: deny all)
- [ ] #3 Justfile: 'just audit' runs cargo-audit && cargo-deny check
- [ ] #4 CI workflow (task-0163) runs 'just audit' weekly on a schedule
<!-- AC:END -->
