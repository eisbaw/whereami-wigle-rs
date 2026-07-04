---
id: TASK-0106
title: 'address_cache_key: hard-guard against NaN/Inf, not debug-only assert'
status: Done
assignee:
  - '@mark'
created_date: '2026-05-20 21:11'
updated_date: '2026-07-04 17:21'
labels:
  - robustness
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/server/address_cache.rs:32-39. address_cache_key quantizes to 1e-4° via 'as i32', then debug_assert\!s finiteness. debug_assert is compiled out in release. Today trilaterate doesn't produce NaN, but the property isn't statically enforced anywhere; a future code change could feed NaN through and the cache would silently insert under (0,0).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 AddressCache::get and AddressCache::insert return early (None / no-op) when lat or lon is not finite
- [x] #2 Replace debug_assert\! with a real assertion at the function boundary, OR an Option<(i32,i32)> return for the key helper
- [x] #3 Unit test: get(NaN, NaN) returns None and never mutates cache state; insert(NaN, ...) is a no-op
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. AC#2: change address_cache_key to return Option<(i32,i32)> — None when lat or lon is non-finite; drop the debug_assert!. This makes the guard hold in release (debug_assert is compiled out).
2. AC#1: get() uses `let key = address_cache_key(lat,lon)?;` (early None). insert() uses `let Some(key) = address_cache_key(lat,lon) else { return; };` (no-op).
3. Update the one production caller server.rs:897-898 to be fail-closed: compare the two Options via .zip().is_some_and(|(a,b)| a==b) so two non-finite keys (None==None) do NOT count as a match. Update the two in-file tests (rounds_to_grid, expires_after_ttl) to unwrap the finite keys.
4. Doc-comment: rewrite address_cache.rs:24-31 (the stale `f64 as i32 saturates ... harmless` rationale) to describe the new explicit Option guard.
5. AC#3: unit test — get(NaN,NaN)/get(Inf,..) return None and leave map+order length unchanged; insert(NaN,..)/insert(..,Inf) are no-ops (map+order length unchanged).
6. Gate: nix develop -c just e2e + just fmt.
Decision: fail-closed SILENTLY (no warn!) in the cache — task-0227 already warns at the server boundary naming the bssid; get/insert run per-locate so a warn here would spam the hot path.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Replaced the debug-only finiteness assert (compiled out in release) with a real Option guard.

Changes:
- address_cache_key now returns Option<(i32,i32)> — None on any non-finite input; debug_assert! dropped (AC#2).
- get(): `self.map.get(&address_cache_key(lat,lon)?)?` -> non-finite reads as a miss (AC#1).
- insert(): `let Some(key) = address_cache_key(lat,lon) else { return; };` -> non-finite is a no-op (AC#1).
- Doc comment (24-31) rewritten: was a STALE rationale ("f64 as i32 saturates ... harmless because trilateration never produces NaN today"); now documents the explicit fail-closed Option guard. The old comment would have been a lie after this fix.
- server.rs:897 caller made fail-closed: address_cache_key(fix)... .zip(address_cache_key(bg)...).is_some_and(|(a,b)| a==b). Prevents None==None (two non-finite positions) from spuriously matching and backfilling an address onto a garbage fix.
- In-file tests (rounds_to_grid, expires_after_ttl) updated to .unwrap() the now-Option keys.

Tests (AC#3):
- address_cache_key_none_on_non_finite: NaN/+Inf/-Inf in either arg -> None; finite -> Some.
- address_cache_non_finite_is_inert: get(NaN,NaN)/get(_,Inf) -> None; insert(NaN,..)/insert(..,-Inf)/insert(Inf,..) leave map.len()==1 and order.len()==1; the real finite entry survives unclobbered.

Decision (lesson): chose fail-closed SILENT (no warn!) in the cache. get/insert run per-locate on the hot path, so a persistently non-finite coord would spam warn!. Observability lives at the server boundary (task-0227) which warn!s naming the bssid. AC#2 explicitly allows the Option-return path with no logging requirement.
Gotcha: a real assert!/panic here was rejected on purpose — a cache helper must not crash the daemon on a bad float.

Gate: just fmt clean (rustfmt required wrapping one assert_eq!); just e2e green; clippy -D warnings exit 0.

Implemented in commit 0ba72ff (batched with task-0227).
- AC#1: AddressCache::get returns None and insert is a no-op when lat/lon is non-finite (early-return on the None key).
- AC#2: address_cache_key now returns Option<(i32,i32)> (None on non-finite), replacing the release-compiled-out debug_assert!. The stale doc comment that rationalized the f64-as-i32 saturation to (0,0) was rewritten to describe the explicit fail-closed guard (comments-must-not-lie).
- AC#3: address_cache_non_finite_is_inert asserts get(NaN,NaN)=None and that map/order length is unchanged after non-finite get/insert; address_cache_key_none_on_non_finite pins the Option contract.
CALLER FIX (same commit): the last_fix address-backfill site in server.rs compared two keys with ==; with Option keys, None==None would spuriously "match" two non-finite positions. Changed to .zip(..).is_some_and(|(a,b)| a==b) (fail-closed). Review noted this is defense-in-depth in the committed state (bg key is always finite post-finite_position, so only fix can be None), and it WAS a live bug before task-0227 existed; comment softened to state the local reachability precisely.
Review gate (measured): qa GO (319 tests, clippy/fmt clean), mped GO.
GOTCHA / forward-carry: changing a key helper from a tuple to Option silently changes == semantics at any call site that compares two keys — audit for None==None. Non-finite fail-closed in the cache is SILENT (no warn!) by design to avoid hot-path log spam; the server-boundary warn! (task-0227) covers observability.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Replaced a release-compiled-out debug_assert in address_cache_key with a real fail-closed guard.

Problem: address_cache_key quantized (lat,lon) to an (i32,i32) grid key via f64-as-i32 and used debug_assert! for finiteness — compiled out in release. A NaN/Inf coordinate would saturate to 0 and silently cache/read an address under the (0,0) cell.

Changes (commit 0ba72ff):
- address_cache_key returns Option<(i32,i32)> (None on any non-finite input); AddressCache::get treats None as a miss, insert as a no-op.
- The doc comment rationalizing the saturation is replaced with the explicit-guard description.
- The last_fix address-backfill caller in server.rs is made fail-closed: two keys are compared with .zip(..).is_some_and(|(a,b)| a==b) so a non-finite key (None) can never spuriously match.

User impact: a non-finite position can no longer read or mutate address-cache state; robustness against a future code path that feeds NaN through.

Tests: address_cache_non_finite_is_inert (get/insert inert + state unchanged), address_cache_key_none_on_non_finite; full gate 319 tests, clippy -D warnings, fmt clean. Review: qa GO, mped GO.
<!-- SECTION:FINAL_SUMMARY:END -->
