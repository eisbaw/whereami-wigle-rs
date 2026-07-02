---
id: TASK-0206
title: Expose the six daemon flags the NixOS module cannot set
status: To Do
assignee: []
created_date: '2026-07-02 04:53'
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
