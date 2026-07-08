---
id: TASK-0127
title: 'accuracy_m wire field semantics: clarify or rename'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:19'
updated_date: '2026-07-08 21:36'
labels:
  - docs
  - wire-format
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Compass-domain finding #6. trilaterate.rs:460-485 estimate_accuracy returns avg_spread.clamp(10.0, 1000.0) — weighted mean distance from centroid to inputs. Not a confidence interval. Public-API convention (W3C Geolocation, Google Maps) is accuracy = 68% or 95% confidence radius. Single-AP returns literal 100.0; antipodal fallback returns 1000.0 as in-band sentinel ('ambiguous') indistinguishable from a legitimately-wide fix. LocateResponse ships this on the wire as 'accuracy_m'.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Choose option A or B and document the choice:
- [x] #2 A) Keep wire name 'accuracy_m' but doc-comment explicitly says: 'weighted spread of inputs around centroid, clamped to [10, 1000]; NOT a calibrated confidence radius'
- [ ] #3 B) Rename wire field 'position_spread_m'; bump protocol version (v=2); client adapts; CHANGELOG note
- [x] #4 Either option: split the antipodal-fallback case into a separate field (fallback: bool OR accuracy_band enum) so 1000.0 sentinel is no longer in-band
- [x] #5 Test: antipodal-input locate returns fallback=true with accuracy_m=1000; legitimate ±900m fix returns fallback=false with accuracy_m=900
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add pub fallback: bool to Position (trilaterate.rs:104) with doc-comment; set at all 3 return sites (single-AP false, antipodal true, weighted-mean false); finite_position passes through.
2. Update finite_position doc/no change needed; fix compile.
3. server.rs: add fallback:bool to LocateResponse with serde skip-when-false; thread pos.fallback via the (lat,lon,accuracy_m,address) tuple (CurrentTrilat=pos.fallback, LastFix=false); stale-fallback ctor false. LastFix persistence unchanged.
4. client lib.rs LocateResponse: add #[serde(default)] pub fallback: bool; fix main.rs literal ctors; minimal render note.
5. Update 0152 cross-test (primary fixture stays fallback-free; add fallback=true fixture/assert). docs/protocol.md + CHANGELOG.
6. Tests: trilaterate antipodal -> fallback==true & accuracy_m==1000; legit multi-AP -> false. server dispatch-level test antipodal cached APs -> fallback:true in JSON.
7. Gate: nix develop -c just e2e + cargo fmt --check.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented (commit 97e11bc). Option A chosen (kept wire name accuracy_m, no protocol bump); added honest doc-comments; AC#4 split via new fallback:bool.

Gotchas found:
- The pre-existing test antipodal_aps_falls_back_to_strongest (-40/-80 signals) did NOT actually exercise the antipodal guard: unequal weights give vector-sum mag 0.0099 >> threshold sum_weight*1e-6 (~1e-8), so it resolved via the WEIGHTED-MEAN path (fallback=false), coincidentally landing on the strong AP with accuracy clamped to 1000. The guard fires ONLY on (near-)equal weights, and integer dBm means the smallest signal gap (1 dBm) is a ~12% weight ratio — far above 1e-6. So a determinate-strongest + guard-fire is impossible; the guard fires only on exact weight ties. Renamed/repurposed that test to document the weighted-mean behavior (asserts fallback=false), and added antipodal_equal_weight_triggers_fallback (both -50) for the true guard path.
- Antipodal fixture used: two APs at (0,0) and (0,180), equal signal -50, for genuine guard firing.
- Persistence: LastFix has no fallback field; new_fix ctor reads only lat/lon/accuracy_m from pos, so fallback is response-only and ignored by persistence (confirmed, no change needed).
- 0152 cross-test: primary LOCATE_FIXTURE left fallback-free (skip-when-false); added assert !r.fallback (defaults) + new LOCATE_FALLBACK_FIXTURE with fallback:true.

Gate: nix develop -c just e2e PASSED. Test-result counts: whereamid lib 466 passed/0 failed; whereamid unit(bin) 205; client lib 50; client main 40; proptests 12; shutdown 4; plus small binaries (2,1,2,0s). clippy --all-targets -D warnings clean. cargo fmt --all --check clean. Left In Progress for review gate; ACs not checked.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Moved the antipodal-cancellation sentinel out of the accuracy_m band and documented accuracy_m honestly (commit a6fb574). DECISION: Option A (keep wire name accuracy_m, NO protocol bump) + the AC#4 fallback split. AC#3 (Option B rename to position_spread_m + v=2) intentionally NOT taken.

Problem: trilaterate returned accuracy_m=1000 as an IN-BAND "ambiguous" sentinel on the antipodal-cancellation fallback path, indistinguishable on the wire from a legitimately-wide fix that also clamps to 1000.

Changes: Position gains pub fallback: bool (set true ONLY at the antipodal return, false at single-AP + weighted-mean; compiler-enforced complete — 3 ctor sites). handle_locate threads pos.fallback into a NEW response-only LocateResponse.fallback with #[serde(skip_serializing_if=Not::not)] (omitted when false); stale/LastFix path sets false. Client LocateResponse gains #[serde(default)] pub fallback; render_locate appends "(ambiguous/fallback fix)" when true. Honest accuracy_m doc-comments added (daemon Position + both LocateResponses + client + protocol.md): "weighted spread clamped [10,1000], NOT a calibrated confidence radius" + (review LOW-3 fix) noting the single-AP path returns a fixed 100.0. CHANGELOG updated.

Test correction (valuable): the pre-existing antipodal_aps_falls_back_to_strongest test (signals -40/-80) NEVER fired the guard — unequal integer-dBm weights give vector-sum magnitude ~0.0099 >> the ~1e-8 guard threshold, so it resolved via the WEIGHTED-MEAN path and coincidentally clamped to 1000. Renamed to antipodal_aps_unequal_weight_resolve_via_weighted_mean_not_fallback (asserts fallback=false, accuracy_m=1000) + added antipodal_equal_weight_triggers_fallback for the genuine guard + a dispatch-level locate_antipodal_cached_aps_report_fallback_true + a client LOCATE_FALLBACK_FIXTURE cross-test.

Persistence: LastFix has no fallback (response-only concept); new_fix reads only lat/lon/accuracy_m — verified.

Verification: dual gate. qa GO (782 tests 0 failed; clippy -D warnings + fmt clean; 5 new/renamed tests meaningful + stable x2). mped GO (guard math verified BY HAND: -40/-80 mag 0.0099 vs threshold 1.01e-8 -> no guard, no regression; -50/-50 -> guard fires; rename is honest not masking a bug; threading compiler-complete; backward-compat accurate all 4 directions; two 1000s now distinct on the wire).

Follow-ups filed: 0262 (the antipodal guard fires only on ~exactly-equal integer-dBm antipodal APs, so fallback is near-dead in practice — decide widen-vs-accept), 0263 (a persisted fallback fix loses the sentinel on later STALE reads — re-introduces in-band ambiguity on the stale path only).
<!-- SECTION:FINAL_SUMMARY:END -->
