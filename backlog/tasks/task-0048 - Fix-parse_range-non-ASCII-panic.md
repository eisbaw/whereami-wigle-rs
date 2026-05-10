---
id: TASK-0048
title: Fix parse_range non-ASCII panic
status: To Do
assignee: []
created_date: '2026-05-10 10:51'
labels:
  - bug
  - history
  - robustness
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
history.rs:126 split_at(spec.len() - 1) indexes by byte. A spec like '7日' (multi-byte unit char) panics on non-char-boundary before the error path runs. handle_history validates via this function; the panic happens before the error response can be produced. Found in v0.4.0 review (mped-architect).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 parse_range never panics on any UTF-8 input
- [ ] #2 Non-ASCII unit chars produce a typed Err with a useful message
- [ ] #3 Property test feeds arbitrary UTF-8 strings and asserts panic-freedom plus shape (Ok or Err)
<!-- AC:END -->
