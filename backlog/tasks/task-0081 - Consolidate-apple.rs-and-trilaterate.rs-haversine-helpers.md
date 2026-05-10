---
id: TASK-0081
title: Consolidate apple.rs and trilaterate.rs haversine helpers
status: Done
assignee: []
created_date: '2026-05-10 11:03'
updated_date: '2026-05-10 14:11'
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
- [x] #1 One canonical haversine_m in a geo / math module, used by trilaterate, history, and proptests
- [x] #2 All three call sites import from the canonical module
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Created whereamid/src/geo.rs with pub fn haversine_m. Exported via lib.rs (pub mod geo) and main.rs (mod geo). The three private copies (trilaterate.rs:223, history.rs:109, proptests.rs:13) replaced with use crate::geo::haversine_m. Added two tests in geo.rs: Copenhagen<->Malmö (~28km) and identity-distance-zero.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Single canonical haversine_m in whereamid::geo. Three duplicate copies removed; consumers re-import via crate::geo::haversine_m. Tests cover round-trip identity and a real-world reference distance.
<!-- SECTION:FINAL_SUMMARY:END -->
