---
id: TASK-0117
title: Drop dead/always-zero fields from LocateResponse wire shape
status: To Do
assignee: []
created_date: '2026-05-20 21:14'
labels:
  - refactor
  - wire-format
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/server.rs:197-218. LocateResponse has nine integer fields (sources, cached, fetched, pending, visible, stable, …) — most of which are zero on most calls. fetched is always 0 (locate is cache-only; fetching happens in background). pending is always 0. These are dead wire fields leaking the old protocol shape that no longer reflects daemon behavior. Clients reading resp.fetched and expecting it to be meaningful are getting useless data.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Audit each integer field for is-it-ever-non-zero in the current code
- [ ] #2 Drop fields proved-dead (likely fetched, pending, possibly others)
- [ ] #3 Optional: move remaining counts into an optional counts sub-object so future fields don't grow the response struct
- [ ] #4 Update docs/protocol.md to match
- [ ] #5 Wire-format test: existing JSON shape minus the dropped fields decodes cleanly on the client side (test_state and proptests pass)
- [ ] #6 CHANGELOG note about wire field removal (backward-compatible since clients with serde(default) treat absent as 0)
<!-- AC:END -->
