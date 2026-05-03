---
id: TASK-14
title: 'Wire up main.rs: daemon assembly'
status: Done
assignee: []
created_date: '2026-05-02 00:06'
updated_date: '2026-05-02 00:20'
labels: []
dependencies:
  - TASK-2
  - TASK-3
  - TASK-9
  - TASK-10
  - TASK-11
  - TASK-12
  - TASK-13
references:
  - PRD.md
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Wire everything together in main.rs: parse CLI args (config.rs), open DB (db.rs), load secrets (config.rs), spawn background scan loop (scanner + debounce), spawn pending queue drain task (pending.rs), start TCP server (server.rs). Shared state via Arc: DB handle, debouncer, config. Signal handling for graceful shutdown (SIGTERM/SIGINT).
<!-- SECTION:DESCRIPTION:END -->
