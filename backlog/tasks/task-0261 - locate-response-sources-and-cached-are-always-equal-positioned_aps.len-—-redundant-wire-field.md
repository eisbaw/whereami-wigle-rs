---
id: TASK-0261
title: >-
  locate response: sources and cached are always equal (positioned_aps.len) —
  redundant wire field?
status: Done
assignee:
  - '@myself'
created_date: '2026-07-08 21:18'
updated_date: '2026-07-09 05:14'
labels:
  - wire-format
  - tech-debt
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Observed during task-0117 review (mped-architect, pre-existing, out of 0117 scope). In handle_locate, both LocateResponse.sources (source_count) and .cached (cached_count) are set to positioned_aps.len() on the PRIMARY path (server.rs ~1961-1962), so `sources == cached` always on a fresh fix (on the stale-fallback ctor cached is 0 and sources is fix.sources). This is the same "redundant/duplicative wire field" class task-0117 addressed (it dropped fetched/pending). Investigate whether `cached` and `sources` are genuinely distinct signals or one is redundant: if they are always equal on every non-stale path, consider dropping one (or renaming to clarify the distinction — e.g. cached = APs that came from cache vs sources = APs that contributed to the fix). AUDIT first (they MIGHT diverge in some path — e.g. if a fix uses fewer sources than cached APs after filtering); only collapse if provably identical. Wire change (client + docs + 0152 fixture) if a field is dropped; backward-compatible via serde(default) like 0117. LOW priority.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Audit recorded: determine whether LocateResponse.sources and .cached are ever distinct (they diverge on the stale-fallback path: sources=fix.sources, cached=0) or always equal
- [x] #2 Since they are distinct (not redundant), document the sources-vs-cached distinction on the wire fields rather than dropping one; no wire change
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
AUDIT RESULT: NOT redundant — sources and cached are distinct signals. Fresh path (server.rs:1643-1644): cached_count = source_count = positioned_aps.len(), so they coincide (locate is cache-only per task-0117, so every contributing AP is a cache hit). Stale-fallback path (server.rs:2116-2117): sources = fix.sources (the historical fix\x27s source count), cached = 0 (no current cache AP contributed). So they diverge on stale-fallback; dropping either would lose that distinction. Resolution: DOCUMENT the distinction on the wire fields (whereami-protocol LocateResponse.sources/.cached), no wire change. Verified: fmt clean, clippy -D warnings clean (intra-doc links resolve). Doc-only; audit is objectively verifiable from the two code paths, so verified in-thread.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Audited whether LocateResponse.sources and .cached are redundant (task-0117 review had noted they are both positioned_aps.len() on the fresh path). Result: they are DISTINCT, not redundant — they coincide on the fresh cache-only path but diverge on the stale fallback (sources = historical fix.sources, cached = 0). Documented the distinction on both wire fields (cached = contributing APs sourced from the local cache; equals sources on a fresh cache-only locate, 0 on a stale fix from the persisted last_fix). No wire change, since dropping either would lose the stale-fallback signal. Doc-only; fmt + clippy clean.
<!-- SECTION:FINAL_SUMMARY:END -->
