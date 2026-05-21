---
id: TASK-0130
title: Rename api_calls -> wigle_lookups (WiGLE-specific terminology)
status: To Do
assignee: []
created_date: '2026-05-20 21:20'
labels:
  - docs
  - wigle
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Compass-domain finding #9. db.rs:670 try_reserve_api_call / refund_api_call. Generic 'api_call' name doesn't say which provider's quota. Only WiGLE actually has one (Apple is unmetered). config.rs:60 --daily-limit is also generic. The counter is implicitly WiGLE-specific but named globally. Future second metered provider would need a (provider, counter) schema.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Rename metadata key api_calls_today -> wigle_lookups_today (migration v6 renames the key, preserves the value)
- [ ] #2 Rename functions try_reserve_api_call -> try_reserve_wigle_lookup, refund_api_call -> refund_wigle_lookup
- [ ] #3 CLI flag: --daily-limit -> --wigle-daily-limit (--daily-limit accepted as alias for one release with a deprecation warning)
- [ ] #4 Stats response field api_calls_today -> wigle_lookups_today (wire-compat: emit BOTH for one release)
- [ ] #5 Doc-comment notes: we mirror WiGLE's server-side counter optimistically; drift possible if other clients share credentials
<!-- AC:END -->
