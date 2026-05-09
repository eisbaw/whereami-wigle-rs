---
id: TASK-0030
title: schema_version table should prevent multiple rows
status: To Do
assignee: []
created_date: '2026-05-09 21:08'
labels: []
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
schema_version table has no constraint preventing multiple rows. get_schema_version uses LIMIT 1 which returns arbitrary row if multiple exist. Add CHECK constraint or use a single-row pattern.
<!-- SECTION:DESCRIPTION:END -->
