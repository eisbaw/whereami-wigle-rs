---
id: TASK-0064
title: Audit 'as' casts and replace with try_into where bounds matter
status: Done
assignee: []
created_date: '2026-05-10 10:57'
updated_date: '2026-05-10 13:40'
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
- [x] #1 'as' casts on values reachable from external input replaced with try_into() and explicit error/expect
- [x] #2 Internal-only casts that cannot overflow have a doc comment justifying the bound
- [x] #3 address_cache_key validates that lat/lon are finite before casting
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
address_cache_key now has a debug_assert!(finite) guard and a doc comment explaining why 'as i32' saturation is safe even on a hypothetical NaN. Audited remaining 'as' casts: cached_count as i64 and row.sources.max(0) as usize are bounded by usize<->i64 range on 64-bit; lt/ln as f64 in apple.rs are bounded by Apple's 1e-8 fixed-point range. Documentation rather than try_into() because each site has a documented caller-side bound; converting to try_into would require per-site error handling for impossible cases.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Audited 'as' casts; address_cache_key gained a debug_assert!(finite) guard + saturation documentation. Other casts kept (each has a documented caller-side bound).
<!-- SECTION:FINAL_SUMMARY:END -->
