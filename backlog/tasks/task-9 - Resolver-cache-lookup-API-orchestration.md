---
id: TASK-9
title: 'Resolver: cache lookup + API orchestration'
status: Done
assignee: []
created_date: '2026-05-02 00:06'
updated_date: '2026-05-02 00:20'
labels: []
dependencies:
  - TASK-2
  - TASK-6
  - TASK-7
  - TASK-8
references:
  - PRD.md
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Implement resolver.rs: given a list of stable BSSIDs, (1) check aps cache, (2) check not_found cache, (3) for remaining misses: if can_call_api(), query WiGLE; on success write to aps table; on 404 write to not_found; on network error or 429 write to pending; (4) if WiGLE quota exhausted, fall back to BeaconDB batch. Returns resolved positions + counts (cached/fetched/pending). Does NOT write to aps from resolve command (only from locate path and pending drain).
<!-- SECTION:DESCRIPTION:END -->
