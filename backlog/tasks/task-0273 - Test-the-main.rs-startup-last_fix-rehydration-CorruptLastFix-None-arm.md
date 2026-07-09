---
id: TASK-0273
title: Test the main.rs startup last_fix rehydration CorruptLastFix->None arm
status: To Do
assignee: []
created_date: '2026-07-09 04:19'
updated_date: '2026-07-09 04:20'
labels:
  - test
dependencies:
  - TASK-0157
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
task-0157 review LOW: main.rs:130-140 maps a CorruptLastFix from get_last_fix() to None + warn during startup rehydration. This binary-entrypoint match arm has no automated test. Risk is LOW because the underlying typed-error behavior is covered at the db layer (get_last_fix_on_corrupt_timestamp_is_err_not_stale) and server layer (locate_with_corrupt_persisted_last_fix_still_answers); the main.rs arm is thin glue reusing the same downcast pattern. Cover the startup-rehydration path so a refactor to .expect() (which would crash the daemon on a corrupt persisted last_fix) is caught.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A test exercises the startup last_fix rehydration path with a corrupt persisted last_fix and asserts it resolves to None + a warning (no panic), not a crash
<!-- AC:END -->
