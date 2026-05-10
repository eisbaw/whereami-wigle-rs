---
id: TASK-0077
title: Defend metadata daily-counter against garbage values
status: To Do
assignee: []
created_date: '2026-05-10 11:01'
labels:
  - db
  - robustness
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
db.rs api_calls_today does String::parse().unwrap_or(0). If anyone pokes the metadata table by hand and writes garbage, the counter silently resets to 0 and the daemon happily re-charges its WiGLE quota. Add CHECK constraint on metadata rows for the rate-limit keys, OR a typed accessor that errors loudly. Found in v0.4.0 review (swe-gardener).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Garbage value in metadata.api_calls_today produces a typed error or a loud warn (not silent reset to 0)
- [ ] #2 Test: insert 'not-a-number' into metadata, call api_calls_today; assert behavior
<!-- AC:END -->
