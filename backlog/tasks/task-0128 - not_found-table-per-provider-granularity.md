---
id: TASK-0128
title: 'not_found table: per-provider granularity'
status: To Do
assignee: []
created_date: '2026-05-20 21:20'
labels:
  - correctness
  - domain
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Compass-domain finding #7. db.rs:434 not_found is keyed on (bssid) only. is_not_found returns a single bool. 'Not found by WiGLE today' is not evidence it's unknown to Apple. NotFoundPolicy::AtChainEnd partly compensates (write only when EVERY provider said NotFound) but the granularity of 'which provider failed' is gone. Adding a new provider can't naturally invalidate stale rows — would need a migration.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Schema migration v6: not_found keyed on (bssid, provider) composite PK, with checked_at timestamp
- [ ] #2 Resolver chain's AtChainEnd rule becomes: 'all currently-configured providers have a fresh row for this BSSID'
- [ ] #3 Adding a new provider naturally re-resolves everything (no row exists for the new provider yet) — no future migration needed
- [ ] #4 Migration backfills existing not_found rows with provider='apple+wigle' or similar consensus marker so the chain treats them as terminal until next check
- [ ] #5 Stats: not_found_per_provider counts exposed (already useful for ops)
- [ ] #6 Test: insert not_found(bssid, apple); is_not_found_for_chain(bssid, [apple]) is true; for chain [apple, wigle] is false (one provider hasn't tried)
<!-- AC:END -->
