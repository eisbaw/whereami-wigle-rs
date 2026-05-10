---
id: TASK-0064
title: Audit 'as' casts and replace with try_into where bounds matter
status: To Do
assignee: []
created_date: '2026-05-10 10:57'
labels:
  - cleanup
  - robustness
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Numerous 'as' casts: server.rs:42 (lat * scale).round() as i32 (saturates on NaN/inf); server.rs:540,549 cached_count as i64; main.rs:72 row.sources.max(0) as usize; apple.rs:190-191 lt as f64 * 1e-8 (lossy at 53 bits); apple.rs:100 proto.len() as u32. Individually fine; collectively an erosion of fail-fast. Found in v0.4.0 review (mped-architect).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 'as' casts on values reachable from external input replaced with try_into() and explicit error/expect
- [ ] #2 Internal-only casts that cannot overflow have a doc comment justifying the bound
- [ ] #3 address_cache_key validates that lat/lon are finite before casting
<!-- AC:END -->
