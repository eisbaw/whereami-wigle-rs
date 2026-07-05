---
id: TASK-0245
title: Persist locate quality metadata in fixes and raw history
status: To Do
assignee: []
created_date: '2026-07-05 14:23'
labels:
  - observability
  - history
dependencies:
  - TASK-0096
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0096 applies the TASK-0094 plausibility policy to autonomous fixes, but the fixes table and TASK-0095 raw-history shape store only timestamp, coordinates, accuracy, and source count. Add durable nullable quality metadata so both manual and autonomous trajectories can expose the confidence and decision context they actually had at collection time. This is a schema/protocol expansion and is deliberately separate from the autonomous-loop refactor.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Add a transactional schema migration with nullable implied_speed_kmh, last_fix_age_secs, inlier_ratio, confidence, and suspect_reason fields; legacy rows remain readable as unknown quality.
- [ ] #2 Persist the selected TASK-0094 decision metadata atomically with every newly appended manual and autonomous fix.
- [ ] #3 Extend raw-history daemon/client JSON additively with optional quality fields while preserving legacy segmented history and older-row compatibility.
- [ ] #4 Document and render unknown versus present quality without fabricating defaults for legacy rows.
- [ ] #5 Add migration, round-trip, legacy-row, manual-fix, autonomous-fix, and raw-wire regression tests.
<!-- AC:END -->
