---
id: TASK-0186
title: >-
  Weighted-centroid weight function: None=>1.0 dominates; unclamped dBm
  overflows to inf/NaN
status: Done
assignee:
  - '@claude'
created_date: '2026-07-02 04:50'
updated_date: '2026-07-02 06:14'
labels:
  - correctness
  - math
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
trilaterate weight = match signal_dbm { Some(dbm) => 10f64.powf(dbm/20.0), None => 1.0 }. None=1.0 equals a 0 dBm signal: 100x the strongest realistic AP, 10000x a weak one, so one signal-less AP owns the centroid. dbm is never range-checked: powf overflows to +inf for large dbm (iw parser saturating cast can produce huge values), and inf/inf normalization yields NaN lat/lon returned as a successful Position. Dormant today (server always fills signal_dbm) but trilaterate is pub and fuzz/proptest exercise the None path. Ref whereamid/src/trilaterate.rs:416-426,470-477.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 a None-signal AP never receives more weight than the strongest measured AP in the same set (e.g. median weight or a nominal -75 dBm)
- [x] #2 trilaterate returns finite lat/lon/accuracy for any i32 signal_dbm (clamp dbm to a physical window)
- [x] #3 one named vote_weight helper is used by both trilaterate and estimate_accuracy
- [x] #4 proptest: mixed Some/None inputs keep the centroid within the cluster diameter bound
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. trilaterate.rs: add DBM_MIN=-120.0, DBM_MAX=0.0 consts + NOMINAL_MISSING_DBM=-75.0.
2. vote_weight(signal_dbm: Option<i32>, fallback_dbm: f64) -> f64: resolve dbm (Some as f64 else fallback), clamp to [DBM_MIN,DBM_MAX], return 10^(dbm/20). Weight always in [1e-6, 1] => finite/positive (AC2).
3. missing_signal_dbm(aps) -> f64: median of present dBm (reuse the now-NaN-safe median()), else NOMINAL. median<=max so a None AP never outweighs the strongest measured (AC1).
4. Replace BOTH weight match arms (trilaterate + estimate_accuracy) with: fallback = missing_signal_dbm(aps) once, then vote_weight(ap.signal_dbm, fallback) in-loop (AC3). Update the stale doc comment.
5. Tests: AC1 (None weight <= strongest measured weight, incl the all-weaker case that a fixed -75 nominal would fail); AC2 (i32::MIN/MAX dbm => finite lat/lon/accuracy); AC4 proptest (mixed Some/None => centroid within cluster diameter + finite).
6. Gate: nix develop -c just e2e + fmt. Commit, no push, no AI attribution.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
forward-carried from task-0185:
- median() in trilaterate.rs is now NaN-safe (sorts via f64::total_cmp). 0186 can safely reuse median() to compute the missing-signal fallback (median of the present dBm values) without any NaN guard.
- rustfmt wraps long assert!/format! messages once they cross ~100 cols (unicode like piR and {interpolation} count); run `nix develop -c just fmt-fix` if `just fmt` (--check) fails.
- whereamid double-counts lib+bin module tests: a unit test added to trilaterate.rs (a lib module compiled into BOTH the lib test binary and the bin test binary) increments the total by 2. proptests.rs (integration) counts once. Current post-0185 totals: client-lib 7, client-bin 10, whereamid-lib 75, whereamid-bin 113, proptests 10 = 215.

Implemented (commit be62037).

- vote_weight(Option<i32>, fallback_dbm) resolves dbm (Some as f64 else fallback), clamps to [DBM_MIN=-120, DBM_MAX=0], returns 10^(dbm/20) in [1e-6, 1] => always finite/positive (AC2). Shared by trilaterate + estimate_accuracy (AC3), fallback computed once per call (avoids O(n^2)).
- missing_signal_dbm(aps) = median of present dBm (reuses the now-NaN-safe median from 0185), else NOMINAL_MISSING_DBM=-75. median<=max, preserved by the monotonic clamp, so None never outweighs the strongest measured (AC1).
- Why median not fixed -75: in a set of only weak APs (all < -75), a fixed nominal would outweigh every real AP. median is <= max by construction. The AC1 test pins exactly this (asserts a fixed -75 WOULD violate the invariant for {-80,-90,None}).
- Updated the stale doc comment ("If no signal info, equal weight (1.0)").

No existing test regressed: all-Some sets are unchanged (clamp is a no-op inside [-120,0]); all-None sets stay equal-weight (constant fallback). Verified test_two_aps_equal_weight, trilaterate_stronger_signal_pulls_centroid, brazil_in_copenhagen still pass.

Gate: nix develop -c just e2e exit 0, fmt exit 0. Test totals 215 -> 220 (+2 unit tests double-counted lib+bin = +4; +1 proptest = +5).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Replaced the trilateration weight match with a single bounded vote_weight helper, fixing two defects on the None / out-of-range paths.

Root cause: weight = match signal_dbm { Some(dbm) => 10^(dbm/20), None => 1.0 }. None=1.0 equals a 0 dBm signal (100x-10000x a real AP) so a lone signal-less AP owned the centroid; and dbm was unclamped, so a saturating-cast value (up to i32::MAX from the iw parser) overflowed 10^(dbm/20) to +inf and inf/inf normalization returned NaN lat/lon as a "successful" Position.

Changes (trilaterate.rs):
- vote_weight(signal_dbm, fallback_dbm): clamps dbm to [-120, 0] before the exponent => finite weight in [1e-6, 1] for any i32 (AC2). Used by both trilaterate and estimate_accuracy so the copies cannot drift (AC3).
- missing_signal_dbm(aps): median of the measured dBm (reuses the NaN-safe median), else -75 when none measured. median <= max => a None AP never outweighs the strongest measured AP (AC1), including the all-weak case a fixed nominal would break.
- Kept the 10^(dbm/20) formula (proportional to 1/distance under free-space path loss).

Tests: None-never-outweighs-strongest, i32::MAX/MIN => finite position, and a mixed Some/None proptest for finiteness + the cluster-diameter bound (AC4).

User impact: latent today (server always fills signal_dbm) but trilaterate is pub and the fuzz/proptest harnesses exercise the None path; this closes a NaN-position and a centroid-domination bug there.

Gate: just e2e + fmt exit 0; 215 -> 220 tests. Risk: none for realistic inputs (clamp is a no-op inside [-120,0], all-None stays equal-weight); behavior only changes for out-of-range dBm and mixed present/absent signals, which is the intended fix.
<!-- SECTION:FINAL_SUMMARY:END -->
