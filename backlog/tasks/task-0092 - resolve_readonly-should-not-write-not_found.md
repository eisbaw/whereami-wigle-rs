---
id: TASK-0092
title: resolve_readonly should not write not_found
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-05-13 21:55'
updated_date: '2026-07-02 18:18'
labels:
  - correctness
  - server
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
resolver.rs::resolve_readonly doc comment says 'ephemeral — does NOT write to aps cache'. That is true about the aps table but misleading: the ChainPolicy has not_found: NotFoundPolicy::PerProvider, so a single WiGLE NotFound is written to the not_found table for not_found_ttl_days (default 30 days). The 'whereami resolve aa:bb:cc:dd:ee:ff' surface is supposed to be 'look this up for me, do not change state'. A user using resolve to probe a BSSID and finding it not in WiGLE now blocks the daemon from re-probing for 30 days — they did not ask for that. Fix: change resolve_readonly's policy to not_found: NotFoundPolicy::Never. Locate and the scan loop already populate not_found proactively; the read-only surface does not need to. Or alternatively: keep the current behavior and update the doc comment to be honest ('ephemeral only applies to positive results'). Pick principle-of-least-surprise: option (a).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 resolve_readonly's ChainPolicy uses NotFoundPolicy::Never
- [x] #2 Doc comment updated to reflect actual behavior (truly ephemeral now)
- [x] #3 Test: call resolve_readonly for an unknown BSSID, assert not_found table is unchanged
- [x] #4 Test: call resolve_readonly for a cached BSSID, assert the result is returned without touching aps or not_found
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. resolve_readonly: flip not_found PerProvider->Never.
2. Remove #[allow(dead_code)] on NotFoundPolicy::Never (now constructed).
3. PerProvider becomes unconstructed -> remove variant + the immediate-write branch in resolve_chain NotFound arm (clippy -D warnings).
4. Fix doc comment: truly ephemeral, writes nothing (no aps/pending/not_found).
5. Tests: readonly-policy + Mock NotFound -> not_found table unchanged (AC3); cached BSSID -> returned, no writes to aps/not_found (AC4).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented in commit 8b2197e (whereamid/src/resolver.rs).
- resolve_readonly policy flipped PerProvider -> Never; extracted readonly_policy() as SSOT so AC#1 asserts on the real policy, not a copy.
- Removed now-dead NotFoundPolicy::PerProvider variant + its immediate-write branch in resolve_chain (clippy -D warnings would flag it). Single-provider immediate-mark == AtChainEnd over one provider, so no behaviour lost.
- Removed #[allow(dead_code)] on Never (now constructed).
Honesty note (AC#2/AC#4 deviation from brief): the brief suggested doc text "writes nothing: no aps, no pending, no not_found". That is FALSE: resolve_chain skip_cached path calls touch_ap on a cache hit, bumping aps.last_seen. Verified in code (resolver.rs cache-hit block). Doc comment now states it writes no positions/pending/not_found but DOES bump last_seen (benign; stored position untouched, not part of ApInfo). AC#4 test asserts the stored position is unchanged and no not_found/pending write, NOT last_seen invariance.
Tests (whereamid bin binary): resolve_readonly_policy_is_never (AC1), resolve_readonly_unknown_bssid_does_not_write_not_found (AC3), resolve_readonly_cached_bssid_returns_without_state_writes (AC4).
Gate: lib 78 / bin 124 / proptests 11 passed; clippy -D warnings clean; cargo fmt --check clean.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
resolve_readonly claimed to be ephemeral but used NotFoundPolicy::PerProvider, so a single WiGLE miss was written to the not_found table for not_found_ttl_days (default 30d) — a user probing a BSSID via `resolve` silently blocked the daemon from re-probing it for a month.

Changes:
- Flipped resolve_readonly to NotFoundPolicy::Never; extracted readonly_policy() as the single source of truth so tests assert on the real policy.
- Removed the now-unconstructed NotFoundPolicy::PerProvider variant and its immediate-write branch (clippy -D warnings). A one-provider chain marking immediately == AtChainEnd over one provider, so nothing was lost.
- Dropped #[allow(dead_code)] on Never (now used).
- Rewrote the doc comment honestly: writes no positions/pending/not_found, but a cache HIT still bumps last_seen via touch_ap (shared bookkeeping; stored position untouched). Did NOT overclaim "writes nothing".

Tests: readonly policy is Never; unknown BSSID through the read-only policy with a Mock NotFound leaves not_found unchanged (AC#3); a cached BSSID is returned with no not_found/pending write and unchanged stored position (AC#4).
<!-- SECTION:FINAL_SUMMARY:END -->
