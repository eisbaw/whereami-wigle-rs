---
id: TASK-0099
title: 'Source::Manual priority below providers silently overwrites user corrections'
status: To Do
assignee: []
created_date: '2026-05-20 21:10'
labels:
  - correctness
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/db/source.rs:39-47. Source::Manual has priority 10, below BeaconDb(20)/Wigle(30)/Apple(40). Doc comment claims Manual is 'lowest deliberately so the system can correct user mistakes by preferring authoritative sources' — but this means a user who manually pins a BSSID position will be silently overwritten on the next provider re-fetch. There's no way to recover user intent from any other source. Today no whereami CLI lets users set Manual, but the priority asymmetry is a latent footgun once that surface ships.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Decide: either add Source::ManualPinned with priority > Apple (e.g. 100), OR add a separate aps.pinned BOOL column gating upsert_ap overwrite
- [ ] #2 If Source::ManualPinned chosen, schema migration v5->v6 with backfill rule (no existing Manual rows -> no-op)
- [ ] #3 Unit test: upsert with Manual(Pinned) blocks subsequent Apple upsert (asserts the user's intent survives)
- [ ] #4 Doc comment on Source explains the design decision plain-English
<!-- AC:END -->
