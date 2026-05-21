---
id: TASK-0112
title: Resolve whereamid lib-vs-binary straddle
status: To Do
assignee: []
created_date: '2026-05-20 21:13'
labels:
  - refactor
  - structure
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
whereamid/src/lib.rs exposes only ~half the daemon's modules (apple, db, debounce, geo, http, scanner, trilaterate). The other half (config, history, nominatim, pending, provider, resolver, server, wigle) is binary-only. Consequences: (1) trilaterate.rs:367 ships #[allow(dead_code)] on geometric_median because history is in the binary; (2) integration tests in whereamid/tests/ cannot reach resolver::resolve_chain, pending::drain_once, or server handlers; (3) #[allow(dead_code)] is climbing.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Decide: 'library + tiny main.rs' (expose all modules in lib.rs) OR 'binary-only' (delete lib.rs, move proptests via pub re-exports from main.rs)
- [ ] #2 Justification recorded in task notes
- [ ] #3 If library-first: pub mod history; pub mod resolver; pub mod pending; pub mod server; etc. in lib.rs. Removes the #[allow(dead_code)] on geometric_median and 10+ other places.
- [ ] #4 Integration tests can now reach the chosen-public modules (verified by adding one new test that calls resolve_chain from whereamid/tests/)
- [ ] #5 If 'binary-only': proptests move to a wrapper or pub re-export mechanism — same observable test surface
<!-- AC:END -->
