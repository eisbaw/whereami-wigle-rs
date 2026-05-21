---
id: TASK-0162
title: 'Test gaps (P2): parse_range trailing ws, address_cache TTL boundary, etc'
status: To Do
assignee: []
created_date: '2026-05-20 21:31'
labels:
  - tests
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Bundle of small test gaps Scout flagged at P2 priority — collected here so each gets a single AC:
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 parse_range("7 d") (with internal whitespace) — currently parses as num='7 ', unit='d' and errors; pin the behavior with an explicit unit test
- [ ] #2 split_nmcli_fields proptest INCLUDES literal backslashes (current proptests exclude them per proptests.rs:168-170 note)
- [ ] #3 address_cache TTL boundary at age==ttl_days exactly: assert get returns None when age >= ttl_days (line address_cache.rs:78 uses >=)
- [ ] #4 handle_locate write-through to last_fix + fixes: end-to-end test that issues locate and asserts both rows after (would have caught task-0046 had it existed pre-fix)
- [ ] #5 Server MAX_RESOLVE_BSSIDS=256 rejection (server.rs:684): fire 257 BSSIDs at handle_resolve, assert error response
<!-- AC:END -->
