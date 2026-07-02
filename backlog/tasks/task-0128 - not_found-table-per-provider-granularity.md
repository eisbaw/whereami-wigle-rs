---
id: TASK-0128
title: 'not_found table: per-provider granularity'
status: To Do
assignee: []
created_date: '2026-05-20 21:20'
updated_date: '2026-07-02 18:35'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from task-0189/task-0194: resolve_chain now builds authoritative_nf: HashSet<String> (BSSIDs a provider authoritatively returned NotFound for, this pass) and ResolveResult.not_found (task-0194). This is the natural seed for per-provider granularity: today the not_found decision is boolean (>=1 provider NotFound AND no hard-transient => mark). To make it per-provider, carry WHICH provider said NotFound (e.g. HashMap<bssid, provider_set>), and make the not_found table + is_not_found() provider-scoped. Motivating case introduced by task-0189: when Apple says NotFound and WiGLE was merely Skipped (unconfigured/quota-exhausted), the chain now marks not_found for the whole TTL; per-provider granularity is exactly what would let the daemon record 'Apple: no' without suppressing a later WiGLE retry. See whereamid/src/resolver.rs chain-end block.

forward-carried from 0189 review: the stopped-provider shadow fix (369429a) makes ALL hard-stop-shadowed BSSIDs transient, which is safe but coarse - per-provider not_found granularity would let Apple-definitive absence be recorded as "apple: no" while WiGLE recovers, resolving both the quota-day 30d-mark tradeoff and the shadow conservatism in one model.
<!-- SECTION:NOTES:END -->
