---
id: TASK-0262
title: 'Decide the fate of the antipodal-fallback guard/field: near-dead in practice'
status: Done
assignee:
  - '@myself'
created_date: '2026-07-08 21:36'
updated_date: '2026-07-09 05:33'
labels:
  - trilaterate
  - design
dependencies:
  - TASK-0127
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
From task-0127 review (mped LOW-1). The two-AP antipodal-cancellation guard (trilaterate.rs, mag < sum_weight*1e-6) fires only when |w1-w2|/(w1+w2) < 1e-6 — with integer-dBm weights the smallest 1 dBm gap is a ~5.75% relative weight difference (>>1e-6), so for two antipodal APs the guard can fire ONLY on IDENTICAL integer dBm (or two signal-less APs). Combined with the physical improbability of genuinely antipodal APs in one scan, fallback=true (and the accuracy_m=1000 sentinel) will be vanishingly rare in production. task-0127 faithfully surfaced this pre-existing property (out-of-band fallback bool) but did not change the guard. DECIDE: does this guard + the new fallback wire field earn their keep? Options: (a) widen the guard threshold so near-antipodal / near-tie ambiguity is actually detected (then fallback becomes meaningful); (b) generalise "ambiguous centroid" detection beyond the exact-antipodal case; (c) accept it as a rare-but-correct safety net and document that fallback is near-always-false. Analysis/design task; low urgency.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Decision recorded (option a widen / b generalise / c accept+document) with the analysis of the guard's firing conditions (fires only on exact-integer-dBm-tie antipodal APs or two signal-less APs)
- [x] #2 Chosen resolution implemented: if (c), document on the guard site AND the LocateResponse.fallback wire field that fallback is near-always-false in production — a rare-but-correct exact-antipodal safety net, not a routine ambiguity signal
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Analyze guard firing conditions (mag < sum_weight*1e-6 with integer-dBm weights) and record in notes.
2. Option (c): add doc-comment at guard site (trilaterate/mod.rs) AND on LocateResponse.fallback (whereami-protocol) stating fallback is near-always-false in production — rare-but-correct exact-antipodal safety net, not a routine ambiguity signal. Note rejection of (a)/(b) as out-of-scope behavioral changes.
3. Doc-only; no math/threshold change. Verify e2e + fmt.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
DECISION: option (c) accept + document (doc-only). Commit 5e5b761.

Guard firing analysis (evidence): the guard is `mag < sum_weight * 1e-6` in whereamid/src/trilaterate/mod.rs. For a two-AP antipodal pair the residual magnitude is |w1-w2|, so the guard needs |w1-w2|/(w1+w2) < 1e-6. Vote weights derive from INTEGER dBm; the smallest 1 dBm gap is a ~5.75% relative weight difference (>> 1e-6). So for two antipodal APs the guard fires ONLY on IDENTICAL integer dBm (or two signal-less APs sharing the missing-signal fallback weight). Combined with the physical improbability of genuinely-antipodal APs in one scan, fallback=true is vanishingly rare in production.

Implemented (c): doc-comment added AT THE GUARD SITE (trilaterate/mod.rs) and on the LocateResponse.fallback wire field (whereami-protocol/src/lib.rs) stating fallback is near-always-false in production — a rare-but-correct EXACT-antipodal numerical safety net, NOT a routine ambiguity signal; callers must not read a widened confidence meaning into its absence.

Rejected (a) widen threshold / (b) generalise ambiguous-centroid detection: both are behavioral changes to the trilateration math, out of scope for this LOW task. The guard is a numerical NaN/garbage-centroid guard, not a confidence heuristic; widening it would change fix behavior and demands its own design + test task. No math/threshold change made.

Gate: `just e2e` 872 passed / 0 failed (baseline 871 + task-0263 test), clippy -D warnings clean, cargo fmt --check clean.

## Review gate (orchestrator, independent) — GO/GO
- DECISION: option (c) accept + document. Options (a) widen threshold / (b) generalise ambiguity detection rejected as behavioral changes out-of-scope for a LOW task.
- Reviewer verified the doc math EXACTLY: vote_weight=10^(dBm/20), so a 1 dBm gap gives relative weight diff (10^0.05-1)/(10^0.05+1)=5.75% >> the 1e-6 guard threshold -> for a two-AP antipodal pair the guard fires only on identical integer dBm (or two signal-less APs). fallback=true is near-always-false in production. Docs added at the guard site (trilaterate/mod.rs) + the LocateResponse.fallback wire field (whereami-protocol). No math/threshold change (guard body untouched, comment only). Dual gate GO/GO (872).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Decided the fate of the two-AP antipodal-cancellation guard + the LocateResponse.fallback wire field: option (c), accept as a rare-but-correct exact-antipodal safety net and DOCUMENT that fallback is near-always-false in production. Analysis: integer-dBm weights (vote_weight=10^(dBm/20)) make the smallest 1 dBm gap a ~5.75% relative weight difference, far above the guard\x27s 1e-6 threshold, so for two antipodal APs the guard fires only on identical integer dBm. Documented this at the guard site and on the wire field so a future reader does not treat fallback as a routine centroid-ambiguity signal. Rejected widening/generalising the guard (behavioral changes, out of scope). Doc-only; dual gate GO/GO. Commit 5e5b761.
<!-- SECTION:FINAL_SUMMARY:END -->
