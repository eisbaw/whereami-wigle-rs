---
id: TASK-0081
title: Consolidate apple.rs and trilaterate.rs haversine helpers
status: To Do
assignee: []
created_date: '2026-05-10 11:03'
labels:
  - refactor
  - math
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Haversine distance is implemented at least 3 times: trilaterate.rs:160 (private), history.rs:97 (private), proptests.rs:13 (test helper). They are identical. Either expose one canonical haversine_m from a small geo.rs module, or accept the duplication and document it. Each duplicate is a place that can drift. Found in v0.4.0 review (swe-gardener).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 One canonical haversine_m in a geo / math module, used by trilaterate, history, and proptests
- [ ] #2 All three call sites import from the canonical module
<!-- AC:END -->
