---
id: TASK-0188
title: >-
  WiGLE success:false (HTTP 200) misread as authoritative NotFound and
  negative-cached
status: To Do
assignee: []
created_date: '2026-07-02 04:50'
labels:
  - correctness
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
WigleSearchResponse deserializes success:bool but it is #[allow(dead_code)] and never checked; the code returns NotFound whenever total_results==0 || results.is_empty(). WiGLE signals query errors and (historically) quota exhaustion as HTTP 200 {success:false, empty results}. Only 429/403 are special-cased, so a transient API failure -> NotFound -> insert_not_found(bssid) -> BSSID skipped for not_found_ttl_days though it was never actually looked up; the reserved quota slot is consumed with no refund. Ref whereamid/src/wigle.rs:37-46,127-137, provider.rs:141-143, resolver.rs:286-291.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 a 200 response with success:false never produces WigleError::NotFound and never writes the not_found table
- [ ] #2 quota-style success:false maps to RateLimited (slot refunded, chain HardStop)
- [ ] #3 unit test with a canned success:false body pins the behavior
<!-- AC:END -->
