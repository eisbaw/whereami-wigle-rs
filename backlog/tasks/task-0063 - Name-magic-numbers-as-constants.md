---
id: TASK-0063
title: Name magic numbers as constants
status: To Do
assignee: []
created_date: '2026-05-10 10:57'
labels:
  - cleanup
  - readability
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Several magic numbers without named constants: pending.rs:43 batch=5 not_found re-check; pending.rs:62 batch=10 pending drain; apple.rs:73,78 header=10 bytes; apple.rs:193 sentinel=-180.0; server.rs:347,736 fallback=-90 dBm (also flagged separately); scanner.rs:46 sleep=1500ms post-rescan. Each should be a const with a doc comment. Found in v0.4.0 review (mped-architect, swe-gardener).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 All listed magic numbers are pub(crate) const with explanatory doc comments
- [ ] #2 Apple constants: APPLE_RESPONSE_HEADER_LEN, APPLE_NOT_FOUND_SENTINEL_LAT (and lon if same)
- [ ] #3 Pending constants: PENDING_DRAIN_BATCH, NOT_FOUND_RECHECK_BATCH
<!-- AC:END -->
