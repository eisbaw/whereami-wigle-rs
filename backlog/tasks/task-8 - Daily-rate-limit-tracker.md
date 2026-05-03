---
id: TASK-8
title: Daily rate limit tracker
status: Done
assignee: []
created_date: '2026-05-02 00:06'
updated_date: '2026-05-02 00:20'
labels: []
dependencies:
  - TASK-2
references:
  - PRD.md
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Implement rate limit tracking in db.rs using the metadata table. Track api_calls_today (counter) and api_calls_date (YYYY-MM-DD UTC). On each API call, check: if api_calls_date != today UTC, reset counter to 0. Increment counter. Method can_call_api() returns bool based on --daily-limit. Method record_api_call() increments. Method api_calls_today() returns current count.
<!-- SECTION:DESCRIPTION:END -->
