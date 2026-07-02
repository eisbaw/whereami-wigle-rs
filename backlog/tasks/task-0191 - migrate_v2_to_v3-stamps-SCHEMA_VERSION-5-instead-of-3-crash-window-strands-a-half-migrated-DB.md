---
id: TASK-0191
title: >-
  migrate_v2_to_v3 stamps SCHEMA_VERSION (5) instead of 3 - crash window strands
  a half-migrated DB
status: To Do
assignee: []
created_date: '2026-07-02 04:50'
labels:
  - correctness
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Both stamp sites in the v2->v3 migration write params![SCHEMA_VERSION] (=5) rather than 3. Normally the in-memory chain heals it microseconds later, but a crash/power-loss/OOM between the v2->v3 commit and the v3->v4 commit leaves the on-disk version at 5 while last_fix (v4) and fixes (v5) do not exist. Every later open reads 5 and skips both migrations: get_last_fix warns, set_last_fix/insert_fix fail on every locate, history returns db errors - permanently, with no self-heal. Ref whereamid/src/db.rs:277-288,308-314 vs chain driver 149-165.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 after migrate_v2_to_v3 commits in isolation the stored schema_version is exactly 3 (keep the version<3 guard)
- [ ] #2 a step-by-step migration test from a v2 fixture observes version stamps 2->3->4->5 with the matching tables present at each step
- [ ] #3 a DB stamped newer than its actual shape is detected (error or repair) instead of silently skipping migrations
<!-- AC:END -->
