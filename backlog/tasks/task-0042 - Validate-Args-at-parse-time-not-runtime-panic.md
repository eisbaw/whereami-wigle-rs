---
id: TASK-0042
title: 'Validate Args at parse time, not runtime panic'
status: To Do
assignee: []
created_date: '2026-05-10 05:38'
labels:
  - config
  - reliability
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
debounce::Debouncer::new asserts threshold <= window and panics if violated. CLI accepts --debounce-threshold and --debounce-window as independent values, so a user can crash the daemon at startup with bad config. Move the validation into a clap value_parser or a custom Args::validate() called immediately after parse, surfacing a clean error before any side effects. Audit other modules for similar runtime panics on user input.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Invalid --debounce-threshold/--debounce-window combinations produce a clean error message at startup, not a panic
- [ ] #2 Audit recorded in task notes: list of all assert!() / panic!() that depend on user-supplied Args values
- [ ] #3 Test covers an invalid combination producing a non-zero exit and a useful message
<!-- AC:END -->
