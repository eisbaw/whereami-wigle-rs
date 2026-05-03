---
id: TASK-12
title: TCP server + JSON-lines protocol
status: Done
assignee: []
created_date: '2026-05-02 00:06'
updated_date: '2026-05-02 00:20'
labels: []
dependencies:
  - TASK-3
references:
  - PRD.md
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Implement server.rs: TCP listener on configurable bind address. One-shot connection lifecycle: accept, read one JSON line, dispatch command, write one JSON response line, close. Commands: locate, resolve, scan, stats. All responses include v:1 field. Parse with serde_json. Handle malformed input gracefully (return error JSON, close). Tokio-based async.
<!-- SECTION:DESCRIPTION:END -->
