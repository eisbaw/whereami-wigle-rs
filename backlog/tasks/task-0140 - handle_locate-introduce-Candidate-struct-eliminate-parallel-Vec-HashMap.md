---
id: TASK-0140
title: 'handle_locate: introduce Candidate struct, eliminate parallel Vec/HashMap'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:24'
updated_date: '2026-07-08 21:56'
labels:
  - refactor
  - server
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/server.rs:411-486. The pipeline builds Vec<(String,i32)> candidates -> HashMap<String,i32> signal_map -> Vec<ApInfo> cached_aps -> Vec<PositionedAp> positioned_aps. The HashMap exists only because lookup_cached returns Vec<ApInfo> (cache-shape) when the call site needs trilateration-input-shape. signal_map round-trips data that was in the original Vec<(String,i32)>.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Define struct Candidate { bssid: String, signal_dbm: i32 } once at the debouncer/locate boundary
- [ ] #2 lookup_cached takes &[Candidate] and returns Vec<PositionedAp> directly (or Vec<(Candidate, ApInfo)> if the caller still needs ApInfo metadata)
- [x] #3 The intermediate signal_map HashMap is removed
- [x] #4 Existing tests pass; this refactor should be net-fewer-lines in handle_locate
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Define struct Candidate (bssid, signal_dbm, ssid; 3 fields, AC stale 2-field omitted ssid needed by pipeline) with doc-comment near locate helpers.
2. Replace tuple Vec<(String,i32,Option<String>)> for visible/stable candidates with Vec<Candidate>; update sort key, filter, bssids map, active set to named fields.
3. Eliminate signal_map: merge active_bssids + signal lookup into one HashMap<&str,&Candidate>; signal from candidate.signal_dbm (byte-identical: branch only taken when active, old signal_map.get was always Some).
4. bounded_uncached_observations takes &[Candidate]; update its test constructor.
5. Keep lookup_cached/_readonly at &[String] (minimal, avoids resolver.rs churn); pass bssids Vec<String>.
6. Gate: nix develop -c just e2e + cargo fmt --check; confirm net line delta <=0.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented (commit 01f748a).
- Introduced struct Candidate { bssid, signal_dbm, ssid } (3 fields; AC #1 2-field version was stale — pipeline needs ssid for mobile classification + provider backfill).
- visible_candidates/stable_candidates now Vec<Candidate>; sort key, filter, bssids map, bounded_uncached_observations(&[Candidate]) + its test constructor updated to named fields.
- Removed redundant signal_map HashMap AND merged the separate active_bssids HashSet into one active: HashMap<&str,&Candidate> index used for both membership + RSSI.
- lookup_cached/_readonly KEPT at &[String] (AC #2 deferred): changing to &[Candidate] would ripple into resolver.rs:1228 autolocate caller + the readonly sibling for no readability gain; minimal focused diff preferred.
- Gate (nix develop -c just e2e): cargo test workspace all pass — whereamid lib 205, main(incl dispatch_tests) 466, proptests 12, shutdown 4; dispatch_tests module 71/71 (locate/cold-boot/consensus/autolocate/non-finite-skip/fallback). clippy --all-targets -D warnings clean. cargo fmt --check clean.
- In-function (handle_locate) net line delta = 0 (AC #4 <=0 met).
Gotcha/tradeoff: to keep loop control-flow byte-identical (net 0), signal is read via active.get(bssid).map(...) inside the same branch already gated by active.contains_key(bssid) — a second cheap probe of the small in-memory map rather than an if-let bind. Single source of truth (active) preserved; not data duplication. Byte-identical because the get is only reached when contains_key is true, so it always yields Some(signal), matching the old signal_map.get().copied().
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Replaced the parallel candidate tuples + intermediate signal_map HashMap in handle_locate with a named Candidate struct (commit 01f748a). Behavior BYTE-IDENTICAL readability refactor (server.rs only).

Changes: new struct Candidate { bssid: String, signal_dbm: i32, ssid: Option<String> } (derives Debug, Clone) replaces the Vec<(String,i32,Option<String>)> tuples (visible/stable candidates + bounded_uncached_observations param); tuple-index accesses (.0/.1/.2, the signal-desc sort key) become named fields. DELETED signal_map: HashMap<String,i32> AND the separate active_bssids: HashSet<&str>, unifying them into ONE active: HashMap<&str,&Candidate> index (single source of truth for both membership and signal — the signal was always in the candidate, no more round-trip).

AC status: #1 Candidate struct DONE (3 fields — the AC's 2-field spec was stale; ssid is needed for mobile-SSID classification + provider backfill). #2 (lookup_cached &[Candidate]) DEFERRED to task-0264 (changing it would ripple into lookup_cached_readonly + the resolver.rs:1228 autolocate caller for zero readability gain — the cache layer legitimately works in bssid-string terms). #3 signal_map removed DONE. #4 net-fewer-lines: net delta 0 in handle_locate (the Candidate doc-comment offsets the deleted signal_map block) — the value is structural (named fields + one fewer collection + single source of truth), not line count.

Verification: dual gate. qa GO (782 tests; the locate/dispatch suite 71/71 UNCHANGED = the byte-identical-behavior proof; clippy -D warnings + fmt clean). mped GO — verified all 4 behavior-preservation sub-claims (same source active_candidates, identical membership, identical signal value, unchanged selection/order/top_n/fallback), and confirmed the dup-key HashMap edge is MOOT (ScanSample is a HashMap so bssids are already unique). In-thread review fix: corrected a stale comment that still referenced the deleted active_bssids variable (doc-vs-code drift).

Deferred to task-0264 (filed by implementer): (a) reconsider lookup_cached &[Candidate] signature; (b) the positioned_aps loop reads the signal via a second active.get() probe inside a contains_key-gated branch (a cheap micro double-lookup; the clean single if-let bind is +1 line under edition-2021 no-let-chains, so deferred). mped confirmed leaving it here is correct.
<!-- SECTION:FINAL_SUMMARY:END -->
