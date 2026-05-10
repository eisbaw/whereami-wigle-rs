---
id: TASK-0038
title: CLI-configurable HTTP timeouts
status: To Do
assignee: []
created_date: '2026-05-10 05:38'
labels:
  - config
  - enhancement
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
task-0020 added explicit HTTP timeouts (5s connect, 15s total fast / 30s Nominatim) but they are compile-time constants in whereamid/src/http.rs. Some users in lossy networks may need longer timeouts; add CLI/config options to override. Single --http-timeout-secs that applies to fast endpoints, plus --nominatim-timeout-secs is enough.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Args has --http-timeout-secs and --nominatim-timeout-secs (or equivalent config keys)
- [ ] #2 PRD updated to reflect that timeouts are now configurable
- [ ] #3 Defaults match the current compile-time constants (15s, 30s)
<!-- AC:END -->
