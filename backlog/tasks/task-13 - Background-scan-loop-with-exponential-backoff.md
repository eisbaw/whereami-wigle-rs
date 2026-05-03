---
id: TASK-13
title: Background scan loop with exponential backoff
status: Done
assignee: []
created_date: '2026-05-02 00:06'
updated_date: '2026-05-02 00:20'
labels: []
dependencies:
  - TASK-4
  - TASK-5
  - TASK-3
references:
  - PRD.md
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Implement in main.rs: tokio::spawn a background scan task. Fast phase: scan every --scan-interval-fast seconds (default 10) for --scan-fast-duration seconds (default 60). Steady phase: scan every --scan-interval-slow seconds (default 60). Each scan calls scanner, pushes results into shared debounce ring buffer (Arc<Mutex<Debouncer>>). Independent of client requests.
<!-- SECTION:DESCRIPTION:END -->
