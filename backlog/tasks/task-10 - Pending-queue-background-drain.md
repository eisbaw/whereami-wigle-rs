---
id: TASK-10
title: Pending queue background drain
status: Done
assignee: []
created_date: '2026-05-02 00:06'
updated_date: '2026-05-02 00:20'
labels: []
dependencies:
  - TASK-2
  - TASK-6
  - TASK-8
references:
  - PRD.md
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Implement pending.rs: background tokio task that runs every --pending-interval seconds (default 300). Each run: pick up to 10 MACs from pending table ordered by attempts ASC. For each: query WiGLE. On success: insert into aps, delete from pending. On 404: insert into not_found, delete from pending. On network error: increment attempts. On attempts >= --pending-max-attempts (default 20): delete from pending. On 429: stop the drain run immediately. Also check not_found table for entries older than 30 days and allow re-query.
<!-- SECTION:DESCRIPTION:END -->
