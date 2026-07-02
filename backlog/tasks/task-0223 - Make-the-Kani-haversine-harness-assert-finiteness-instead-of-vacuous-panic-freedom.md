---
id: TASK-0223
title: >-
  Make the Kani haversine harness assert finiteness instead of vacuous
  panic-freedom
status: To Do
assignee: []
created_date: '2026-07-02 04:55'
labels:
  - testing
  - math
dependencies:
  - TASK-0185
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
haversine_m_no_panic_on_finite_input only calls haversine_m and discards the result. haversine_m has no panicking op (no indexing/unwrap/integer arithmetic), so pure-f64 panic-freedom is true by construction - the proof establishes nothing. The actual defect on this surface is the NaN output (see the haversine NaN task) whose panic lives downstream in median(). Ref whereamid/src/geo.rs:52-66 (uncommitted task-0082 harness).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 the geo.rs harness asserts finiteness/non-negativity of the output under an in-range-degrees assume, or is removed with rationale
- [ ] #2 harness passes after the haversine clamp fix
<!-- AC:END -->
