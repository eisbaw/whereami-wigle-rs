---
id: TASK-0227
title: Fail-fast on non-finite lat/lon entering trilaterate (cache-read guard)
status: Done
assignee:
  - '@mark'
created_date: '2026-07-02 06:34'
updated_date: '2026-07-04 17:21'
labels:
  - correctness
  - math
dependencies:
  - TASK-0185
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Follow-up from the task-0185 review. The median() NaN-proofing (total_cmp) removed the panic, but a NaN/Inf coordinate read from a cached aps row still flows through to_unit_vec -> NaN vector -> the mag>0 guards are false-on-NaN -> from_unit_vec returns (NaN, NaN) as an Ok Position. SQLite REAL columns can hold NaN. task-0185 only constrained finite INPUTS to haversine; non-finite coords entering the pipeline are only symptom-patched. Ref whereamid/src/trilaterate.rs (to_unit_vec/from_unit_vec, entry), resolver.rs lookup_cached / db.rs aps read.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 non-finite lat/lon read for an AP is skipped (not fed to trilaterate) with a warn! naming the bssid
- [x] #2 trilaterate never returns a Position with non-finite lat/lon for any cached-row input
- [x] #3 a unit/proptest test injects a NaN-coord AP and asserts it is excluded, not returned
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. server.rs:830 (AC#1): insert a .filter() before building positioned_aps that drops cached ApInfo rows whose lat/lon is non-finite, emitting warn! naming ap.bssid. warn already imported (server.rs:16).
2. trilaterate.rs:451 (AC#2): at trilaterate entry, after the empty check, filter out non-finite PositionedAps into a local Vec; if that empties, bail! with the same message as the empty case. Rebind aps to the finite slice so filter_outliers, the single-AP fast path (462) and the strongest-AP fallback (498) can only echo finite coords.
3. trilaterate.rs (AC#2): add a small finite_position(Position)->Result<Position> gate (checks lat/lon/accuracy_m finite, else bail!) and route all three Ok returns through it (defense in depth; promotes the test-only asserts at 681-684 into production).
4. Tests (AC#3): unit tests in trilaterate.rs tests mod (lone NaN -> Err; NaN/Inf mixed with finite cluster -> finite result near cluster) + a proptest in tests/proptests.rs injecting NaN/Inf into a finite cluster and asserting the result stays finite.
5. Gate: nix develop -c just e2e (cargo test + clippy -D warnings) and just fmt.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented fail-closed-on-non-finite across the whole coordinate pipeline.

Changes:
- server.rs (~830): .filter() drops cached ApInfo rows with non-finite lat/lon BEFORE building positioned_aps, warn!("skipping AP {bssid}: non-finite cached position ({lat}, {lon})"). First line of defense (AC#1).
- trilaterate.rs entry: filter non-finite PositionedAps into a local `finite` Vec, rebind aps to it; if it empties, bail! with the SAME message as the empty-input case. This makes the single-AP fast path AND the strongest-AP fallback safe (they echo an input coord verbatim). (AC#2)
- trilaterate.rs: new finite_position(Position)->Result gate; all THREE Ok returns route through it. Promotes the cfg(test) asserts at old 681-684 into a production invariant (AC#2).

Tests (AC#3):
- trilaterate.rs: lone_non_finite_ap_errors_not_returned (NaN/+Inf/-Inf in lat or lon on a lone AP -> Err, not Ok(NaN)); non_finite_ap_excluded_from_cluster (poisoned AP with strong -30 dBm mixed into a finite cluster -> result finite AND inside the cluster, proving the bad AP contributed nothing).
- tests/proptests.rs: trilaterate_excludes_non_finite_coord_ap (property: any finite cluster + one NaN/Inf AP (lat or lon) -> finite Ok).

Gotchas / lessons:
- The two fallback return paths were the trap: without the entry filter, filtering only at the centroid path would still let single-AP / strongest-AP echo a NaN input straight into an Ok(Position). Entry-filter + final gate together close all three.
- If ALL cached rows are non-finite, positioned_aps empties and trilaterate returns Err; server degrades via return_last_fix_or_error (server.rs:986) -> no panic. Verified by reading the Err arm.
- finite_position also checks accuracy_m (matches the existing task-0186 invariant test), not just lat/lon.

Gate: just fmt clean; just e2e green (whereamid main 178, lib 88, proptests 12 (was 11), shutdown 3, client 15+22, sigpipe 1; clippy --all-targets -D warnings exit 0).

Implemented in commit 0ba72ff (batched with task-0106).
- AC#1: handle_locate (server.rs) filters cached ApInfo with non-finite lat/lon before building positioned_aps, warn!-ing the bssid.
- AC#2: trilaterate() filters non-finite inputs at entry (covering the single-AP fast path and strongest-AP fallback, which echo an input coord verbatim) and routes all three Ok returns through a new finite_position() gate that bail!s on non-finite; the empty-after-filter bail! is distinct ("all N input APs had non-finite coordinates") so a poisoned batch is distinguishable from an empty one.
- AC#3: trilaterate unit tests lone_non_finite_ap_errors_not_returned + non_finite_ap_excluded_from_cluster (NaN/+Inf/-Inf, poisoned strong-signal AP proven to contribute nothing) + proptest trilaterate_excludes_non_finite_coord_ap (2-8 finite APs + one poisoned).
Review gate (orchestrator-run, measured): qa-test-runner GO = 319 workspace tests pass, clippy -D warnings clean (forced recompile), fmt clean, new proptest stable 5 runs incl. 4096 cases. mped-architect GO. In-thread hardening after review: softened an over-stated keys_match comment; made the empty-after-filter bail! message diagnostic.
GOTCHA / forward-carry: the trap in this pipeline is the TWO fallback return paths (single-AP, strongest-AP) that echo an input coordinate verbatim — filtering only at the centroid path would leave them able to return a NaN input as Ok. Any future task adding a trilaterate return path MUST route it through finite_position and keep the entry filter. finite_position also checks accuracy_m.is_finite(); estimate_accuracy is clamped [10,1000] so it never false-rejects a real fix. mped found the SAME NaN-from-REAL bug class on sibling paths (LastFix rehydration, resolve serve) -> filed task-0238; server-boundary warn/skip is untested -> filed task-0239.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Fail-closed on non-finite (NaN/±Inf) coordinates entering trilateration.

Problem: SQLite REAL columns can hold NaN/±Inf. A non-finite cached lat/lon flowed through to_unit_vec -> NaN vector -> from_unit_vec, returning (NaN, NaN) as an Ok(Position) — a garbage location that looked like a successful fix. task-0185 only constrained finite inputs to haversine; the pipeline entry was unguarded.

Changes (commit 0ba72ff):
- server.rs handle_locate skips cached APs with non-finite coords before trilaterate, warn!-ing the offending bssid (first line of defense + traceability).
- trilaterate filters non-finite inputs at entry — which makes the single-AP fast path and strongest-AP fallback safe because they echo an input coord — and routes every Ok return through a finite_position() gate that bail!s on non-finite, turning the former cfg(test) finiteness asserts into a production invariant.
- New unit tests + a proptest inject NaN/±Inf and assert exclusion, not a returned (NaN,NaN).

User impact: a poisoned AP row can no longer produce a bogus (NaN,NaN) fix; it is dropped with an observable warning.

Tests: 319 workspace tests, clippy -D warnings, fmt clean; new proptest stable across 5 runs incl. a 4096-case sweep. Review: qa GO, mped GO.

Follow-ups filed: task-0238 (same NaN-from-REAL bug class at the LastFix rehydration + resolve serve boundaries, which bypass this guard), task-0239 (handle_locate server-boundary test for the skip+warn).
<!-- SECTION:FINAL_SUMMARY:END -->
