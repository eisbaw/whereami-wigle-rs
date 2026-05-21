---
id: TASK-0171
title: 'Justfile: parameterize fuzz recipe (kill 3-edit-per-target bloat)'
status: To Do
assignee: []
created_date: '2026-05-20 21:34'
labels:
  - tooling
  - fuzz
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Forge F9. Justfile:42-52 and :55-60 are two near-identical 5-line blocks (fuzz, fuzz-all) differing only in -max_total_time=15 vs =60. New fuzz target = 3 edits (fuzz-X, fuzz, fuzz-all).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Parameterized: _fuzz-each SECONDS -> loops over fuzz targets and runs each for {{SECONDS}}
- [ ] #2 fuzz: (_fuzz-each '15'); fuzz-all: (_fuzz-each '60')
- [ ] #3 Adding a new fuzz target = one place (target file + ensure name in the loop's target list)
- [ ] #4 Optional: derive target list from cargo fuzz list output so even the loop list becomes automatic
<!-- AC:END -->
