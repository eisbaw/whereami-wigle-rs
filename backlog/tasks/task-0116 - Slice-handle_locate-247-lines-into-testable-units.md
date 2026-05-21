---
id: TASK-0116
title: Slice handle_locate (247 lines) into testable units
status: To Do
assignee: []
created_date: '2026-05-20 21:13'
labels:
  - refactor
  - server
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/server.rs:397-643. Phases: stable candidate selection (411-417), cold-start fallback (419-436), cached lookup (457-476), trilateration (488), address resolution (501-567), persistence (585-613), response build (615-630). Each phase is testable in isolation; none currently is. task-0058 added ok_json helper but the handler kernel wasn't sliced.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Extract handle_locate_inner(state, candidates: &[(String,i32)]) -> Result<Position> — the pure kernel: cached lookup + filter + trilat. No mutex acquisition, no spawning, no DB writes.
- [ ] #2 Extract persist_locate(state, position, sources) -> Result<()> — last_fix upsert + fixes insert. Single lock-ordering site.
- [ ] #3 Extract resolve_cold_start_fallback(state) -> Vec<(String,i32)> — for the empty-candidates case
- [ ] #4 handle_locate becomes a thin assembler over the three (~30 lines)
- [ ] #5 New unit tests: handle_locate_inner with synthetic candidates returns expected Position without touching the DB beyond cache reads
<!-- AC:END -->
