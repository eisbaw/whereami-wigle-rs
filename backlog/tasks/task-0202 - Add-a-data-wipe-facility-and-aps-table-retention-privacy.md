---
id: TASK-0202
title: Add a data-wipe facility and aps-table retention (privacy)
status: To Do
assignee: []
created_date: '2026-07-02 04:52'
labels:
  - privacy
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
History fixes are pruned by --history-retention-days but there is no user-facing wipe (no subcommand, no just recipe, no documented procedure), and the aps cache (every BSSID/SSID/position ever seen) has no retention/eviction at all. ADR-030 explicitly defers a purge command that was never built. A user needing 'delete everything now' (shared machine, seizure risk) has no supported path. Ref whereamid/src/db.rs, docs/adr/030-cache-first.md:24.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 a single documented command removes all stored location/history/AP rows and the last-known fix (wipe control command + CLI subcommand and/or just recipe)
- [ ] #2 aps rows past a configurable age are pruned by the background sweep
- [ ] #3 README documents retention defaults and the wipe procedure under a Privacy section
<!-- AC:END -->
