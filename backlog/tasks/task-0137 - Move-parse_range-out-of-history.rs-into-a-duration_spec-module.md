---
id: TASK-0137
title: Move parse_range out of history.rs into a duration_spec module
status: To Do
assignee: []
created_date: '2026-05-20 21:22'
labels:
  - refactor
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/history.rs:124-150. parse_range parses s/m/h/d/w duration specs — generic, not history-specific. If task-0031's history feature ever gets siblings (e.g. stats-over-time endpoint), parse_range gets pulled out, breaking history.rs's pub API. Better to extract while there is one caller.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 New module src/duration_spec.rs (or in src/geo.rs alongside haversine helpers — author choice)
- [ ] #2 parse_range moved; history.rs uses 'use crate::duration_spec::parse_range'
- [ ] #3 Existing tests in history.rs (including parse_range_no_panic_on_non_ascii from task-0048) move with the function
- [ ] #4 Kani harness for parse_range_no_panic (task-0082) moves too
<!-- AC:END -->
