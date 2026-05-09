---
id: TASK-0021
title: Deduplicate resolve_background spawns
status: To Do
assignee: []
created_date: '2026-05-09 21:07'
labels: []
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Scan loop spawns resolve_background every tick. Locate fallback also spawns one for the same BSSIDs. Two concurrent tasks hit Apple/WiGLE simultaneously, doubling API consumption. Add a HashSet of currently-resolving BSSIDs behind a Mutex to prevent redundant work.
<!-- SECTION:DESCRIPTION:END -->
