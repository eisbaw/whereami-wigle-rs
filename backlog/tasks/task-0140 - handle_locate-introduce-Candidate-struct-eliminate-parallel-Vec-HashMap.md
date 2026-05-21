---
id: TASK-0140
title: 'handle_locate: introduce Candidate struct, eliminate parallel Vec/HashMap'
status: To Do
assignee: []
created_date: '2026-05-20 21:24'
labels:
  - refactor
  - server
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/server.rs:411-486. The pipeline builds Vec<(String,i32)> candidates -> HashMap<String,i32> signal_map -> Vec<ApInfo> cached_aps -> Vec<PositionedAp> positioned_aps. The HashMap exists only because lookup_cached returns Vec<ApInfo> (cache-shape) when the call site needs trilateration-input-shape. signal_map round-trips data that was in the original Vec<(String,i32)>.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Define struct Candidate { bssid: String, signal_dbm: i32 } once at the debouncer/locate boundary
- [ ] #2 lookup_cached takes &[Candidate] and returns Vec<PositionedAp> directly (or Vec<(Candidate, ApInfo)> if the caller still needs ApInfo metadata)
- [ ] #3 The intermediate signal_map HashMap is removed
- [ ] #4 Existing tests pass; this refactor should be net-fewer-lines in handle_locate
<!-- AC:END -->
