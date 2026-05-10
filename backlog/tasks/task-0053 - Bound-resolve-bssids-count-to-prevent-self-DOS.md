---
id: TASK-0053
title: Bound /resolve bssids count to prevent self-DOS
status: To Do
assignee: []
created_date: '2026-05-10 10:54'
labels:
  - server
  - robustness
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Request.bssids: Vec<String> is unbounded (server.rs:309). A 64KiB JSON of 6-byte BSSIDs is ~7000 entries; resolve_chain takes the DB lock per BSSID and would starve the rest of the daemon. MAX_REQUEST_BYTES bounds the wire size but not the count. Found in v0.4.0 review (mped-architect).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 handle_resolve rejects requests with more than MAX_RESOLVE_BSSIDS (e.g. 256) entries with a clear error
- [ ] #2 Limit is a named const, not a magic number
- [ ] #3 Test for limit-exceeded request
<!-- AC:END -->
