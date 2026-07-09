---
id: TASK-0270
title: 'Update dataflow-persist.drawio schema-version label (stale at 7, now 9)'
status: To Do
assignee: []
created_date: '2026-07-09 02:35'
labels:
  - docs
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
docs/dataflow-persist.drawio labels the schema-version cylinder 'version = 7'. It was already stale at v8 (task-0130) and is now v9 (task-0123 dropped the redundant channel column). Pre-existing doc staleness, flagged during the task-0123 review. Update the label to the current SCHEMA_VERSION and ideally add a note to bump it with future migrations (or derive it).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 The drawio schema-version label matches the current SCHEMA_VERSION (9)
<!-- AC:END -->
