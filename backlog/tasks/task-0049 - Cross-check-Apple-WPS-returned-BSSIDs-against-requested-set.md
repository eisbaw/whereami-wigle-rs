---
id: TASK-0049
title: Cross-check Apple WPS returned BSSIDs against requested set
status: To Do
assignee: []
created_date: '2026-05-10 10:52'
labels:
  - apple
  - robustness
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
apple.rs:135 decode_response ignores _requested. The decoder trusts whatever BSSIDs the server returns and stores them into the cache. If Apple returns a BSSID format mismatch or stray entry, it lands in aps. Low risk in practice — Apple's protocol is well-behaved — but the dead parameter is a documented invariant the code does not enforce. Found in v0.4.0 review (mped-architect).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 decode_response actually consumes _requested as a HashSet/BTreeSet
- [ ] #2 Returned BSSIDs not in the requested set are dropped with a debug! log
- [ ] #3 Test feeds a response containing an unrequested BSSID and asserts it is filtered
<!-- AC:END -->
