---
id: TASK-0156
title: 'Test: Apple stray-BSSID filter (task-0049 regression)'
status: To Do
assignee: []
created_date: '2026-05-20 21:30'
labels:
  - tests
  - apple
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Scout P1-2. apple.rs:169 has 'if requested_set.contains(&ap.bssid)' filter (task-0049). No test feeds a response containing a BSSID NOT in requested and asserts it's dropped. Apple has historically responded with neighbours-of-requested BSSIDs in some configurations.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Hand-build a protobuf response with two WifiDevice entries; pass requested=[bssid_of_first] to decode_response; assert result has exactly 1 entry with the right BSSID
- [ ] #2 Symmetric: requested=[both] -> result has 2 entries
- [ ] #3 Symmetric: requested=[none-matching] -> result is empty
<!-- AC:END -->
