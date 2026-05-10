---
id: TASK-0056
title: Split server.rs into submodules and slice handle_locate
status: To Do
assignee: []
created_date: '2026-05-10 10:54'
labels:
  - refactor
  - server
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
server.rs is 967 lines mixing TCP framing, 8 wire types, AddressCache, DaemonState, LastFix, lock_db helper, and 7 command handlers. handle_locate alone is 250 lines mixing candidate selection, cold-start fallback, cache lookup, trilateration, address backfill spawn, last_fix persistence, history insert. Found in v0.4.0 review (keeper-maintainer, swe-gardener).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 server/{wire,address_cache,handlers/*}.rs split (one file per command)
- [ ] #2 handle_locate decomposed into select_candidates, spawn_address_backfill, persist_fix helpers
- [ ] #3 All existing tests still pass; address_cache tests move to address_cache.rs
<!-- AC:END -->
