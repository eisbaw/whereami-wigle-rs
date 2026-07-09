---
id: TASK-0139
title: 'Add Database::classify_bssid(bssid, ttl_days) -> DbStatus single-query method'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:23'
updated_date: '2026-07-09 04:53'
labels:
  - refactor
  - db
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/server.rs:805-861. handle_debug iterates BSSIDs and calls get_ap + (maybe) is_not_found + (maybe) is_pending per BSSID — up to 3 DB lock acquires × N. Task-0091 is about releasing the debouncer mutex but the underlying anti-pattern persists. Conceptually each BSSID has one DbStatus state (Cached(ApInfo) | NotFound | Pending | New), discoverable in one query.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Database::classify_bssid(bssid, ttl_days) -> Result<DbStatus> with one SQL using LEFT JOIN aps/not_found/pending
- [x] #2 handle_debug calls classify_bssid once per BSSID under one DB lock acquisition
- [x] #3 Existing handle_debug tests pass; new test asserts the single-query semantics by stubbing the DB and counting queries OR checking lock count
- [ ] #4 Resolver chain may also use classify_bssid (collapses the existing get_ap + is_not_found + is_pending probes into one)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add sync Database::classify_bssid(bssid, ttl_days) -> Result<DebugDbStatus>: ONE query, FROM (SELECT ?1) base LEFT JOIN aps/not_found/pending; not_found JOIN carries checked_at > cutoff (cutoff = now - ttl_days, byte-identical to is_not_found); CASE returns aps->Cached, nf->NotFound, pending->Pending, else New (same priority as if-else).
2. debug_statuses calls classify_bssid once per bssid inside its single read() lock (1 SQL x N, was 3).
3. Test classify_bssid_matches_ifelse_priority_and_ttl: New/Cached/NotFound/Pending isolated + aps-wins-over-not_found priority + expired-not_found falls through to New and to Pending (cross-checked vs is_not_found).
4. AC#4 resolver: evaluate; resolver probes are async DbHandle + get_ap needs ApInfo payload => defer unless clean.
5. Gate: nix develop -c just e2e + cargo fmt --check.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented classify_bssid (sync, on inner Database): ONE LEFT JOIN across aps/not_found/pending, CASE returns Cached/NotFound/Pending/New in the SAME priority as the old if-else. not_found JOIN carries checked_at > cutoff (cutoff = now - ttl_days), byte-identical to is_not_found. debug_statuses now calls it once per bssid inside its single read() lock (1 SQL x N, was 3). get_ap + is_pending lost their last production callers -> gated #[cfg(test)] alongside sibling single-row helpers (upsert_ap/insert_pending/insert_not_found). AC#4 (resolver) DEFERRED: resolver probes are async DbHandle round-trips and the cache-hit probe needs get_ap ApInfo payload; classify_bssid is tag-only + sync, so no clean tag-only resolver site to collapse. Gate: nix develop -c just e2e PASS (871 passed, baseline 869 + new test counted in the two suites compiling the db module), clippy -D warnings clean, cargo fmt --check clean. Commit 7958b6f (code only). Left In Progress for review; ACs not checked.

## Review gate (orchestrator, independent) — GO/GO
- QA (2x): 871/871, zero flake; clippy 0 -D warnings (proves the #[cfg(test)]-gated sync get_ap/is_pending have NO production callers — compiler-enforced); fmt clean. classify_bssid test + all 5 debug tests pass.
- Architecture: BYTE-IDENTICAL classification CONFIRMED. (1) CASE order aps(0)>not_found(1)>pending(2)>new(3) matches the old if-else; aps-wins verified (bssid in aps + fresh not_found -> Cached). (2) ttl predicate byte-for-byte: both is_not_found and classify use cutoff=(Utc::now()-TimeDelta::days(ttl_days)).to_rfc3339() + strict `checked_at > ?2`; the predicate is in the LEFT JOIN ON clause so an expired not_found row yields n.bssid IS NULL (falls through), NOT filtering the base row — correct construction. (3) test bites priority AND ttl fall-through (cross-checked vs is_not_found). (3b) all 3 tables have bssid PRIMARY KEY -> each LEFT JOIN <=1 row, base row not multiplied. (4) #[cfg(test)] gating sound (matches sibling helpers upsert_ap/insert_not_found/insert_pending; non-test build compiles clean). (5) AC#4 deferral justified: resolver cache-hit uses lookup_cached_touch -> ApInfo payload for trilateration, which tag-only classify_bssid cannot supply.
- LOW (moot): ttl test uses deep-past (ttl+16d) not the exact microsecond boundary, so it does not independently distinguish >vs>= — but classify & is_not_found SHARE the identical operator+expression, so there is provably no divergence.
- STALE-PREMISE corrected: the task premise (3 DB locks x N) was already fixed by debug_statuses batching to 1 lock; this task collapsed the remaining 3 SQL x N into 1 LEFT JOIN x N inside that lock + extracted the reusable primitive.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added Database::classify_bssid(bssid, ttl_days) -> DebugDbStatus: one LEFT-JOIN query over aps/not_found/pending returning the BSSID\x27s cache classification, replacing the 3-SELECTs-per-BSSID (get_ap + is_not_found + is_pending) inside debug_statuses. handle_debug now runs 1 SQL x N (was 3 x N) within its single DB lock.

Byte-identical classification: the CASE preserves the exact priority (aps>not_found>pending>new) and the not_found JOIN replicates is_not_found\x27s ttl predicate exactly (strict checked_at > cutoff, same cutoff expression), in the ON clause so an expired not_found row falls through rather than filtering the base row. A new priority+ttl test bites both dimensions, cross-checked against is_not_found.

Side effect: collapsing debug_statuses removed the last production callers of the sync get_ap/is_pending, so they are now #[cfg(test)] (compiler-enforced dead-code removal, matching the sibling single-row test helpers). AC#4 (resolver reuse) assessed and deferred: the resolver cache-hit needs get_ap\x27s ApInfo payload for trilateration, which the tag-only classify_bssid cannot supply — no clean adoption (optional per the AC\x27s "may").

Note: the task premise (3 locks x N) was already fixed by debug_statuses batching; this delivered the residual query-count reduction + the reusable primitive.

Tests: 871 workspace (+1), dual gate GO/GO, zero flake. Commit 7958b6f.
<!-- SECTION:FINAL_SUMMARY:END -->
