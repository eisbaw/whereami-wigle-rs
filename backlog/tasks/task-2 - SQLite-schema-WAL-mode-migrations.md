---
id: TASK-2
title: SQLite schema + WAL mode + migrations
status: Done
assignee: []
created_date: '2026-05-02 00:05'
updated_date: '2026-05-02 00:13'
labels: []
dependencies:
  - TASK-1
references:
  - PRD.md
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Implement db.rs: open/create SQLite DB with WAL mode. Create tables: aps, not_found, pending, metadata, schema_version. All timestamps UTC. Include schema version 1. Add migration framework for future versions.
<!-- SECTION:DESCRIPTION:END -->
