---
id: TASK-0157
title: 'Test: LastFix parse-error path doesn''t crash'
status: To Do
assignee: []
created_date: '2026-05-20 21:30'
labels:
  - tests
  - robustness
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Scout P1-3. server.rs:139 TryFrom<LastFixRow> for LastFix errors on unparseable at_rfc3339. main.rs:73-80 uses this as a 'discard malformed last_fix' path. No test exercises an unparseable timestamp. If someone refactors to .expect(), the daemon would crash on startup if garbage ever lands in last_fix.at_rfc3339.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Test: insert a last_fix row via raw SQL with at_rfc3339='garbage'
- [ ] #2 Call LastFix::try_from(row); assert is_err()
- [ ] #3 End-to-end: open Database with corrupt last_fix, assert open succeeds and the in-memory last_fix is None (boot rehydrate path)
<!-- AC:END -->
