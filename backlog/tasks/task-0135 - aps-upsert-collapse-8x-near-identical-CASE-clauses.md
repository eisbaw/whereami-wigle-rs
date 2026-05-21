---
id: TASK-0135
title: 'aps upsert: collapse 8x near-identical CASE clauses'
status: To Do
assignee: []
created_date: '2026-05-20 21:21'
labels:
  - refactor
  - db
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/db.rs:485-510. ON CONFLICT upsert repeats CASE WHEN excluded.source_priority >= aps.source_priority THEN excluded.X ELSE aps.X END for eight columns (lat, lon, ssid, source, source_priority, encryption, channel, frequency, etc.). Adding a column means editing 9 places (table, INSERT param list, 8 CASE clauses). Refactor hazard.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Choose ONE: a) Trigger-based: an AFTER INSERT trigger that UPDATEs rows where the new priority loses, OR b) Two-step: INSERT to temp; UPDATE aps SET (cols) = (temp.cols) WHERE excluded.priority >= aps.priority; DELETE temp
- [ ] #2 Adding a column to aps requires editing the schema and the param list — NOT N CASE clauses
- [ ] #3 All existing upsert behavior preserved (priority-aware overwrite); tests for source priority still pass
- [ ] #4 Add a new column via the new mechanism in this PR (e.g. encryption_type or band) to prove the ergonomics
<!-- AC:END -->
