---
id: TASK-0058
title: Add CLI dispatch_response and server ok_json helpers
status: To Do
assignee: []
created_date: '2026-05-10 10:55'
labels:
  - refactor
  - cleanup
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
whereami-client/src/main.rs has 6 near-identical match ladders (Ok if json => raw JSON; Ok if !ok => fatal; Ok => render; Err => fatal). server.rs has 6+ identical 'serde_json::to_string(&resp).unwrap_or_else(|_| error_json(...))' tails. Two small helpers eliminate ~80 LOC of boilerplate. Found in v0.4.0 review (mped-architect).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 CLI dispatch_response<T>(result, json, render: impl Fn(&T)) used by every subcommand
- [ ] #2 Server ok_json<T: Serialize>(t: &T) -> String used by every handler
- [ ] #3 Net LOC reduction visible in diff
<!-- AC:END -->
