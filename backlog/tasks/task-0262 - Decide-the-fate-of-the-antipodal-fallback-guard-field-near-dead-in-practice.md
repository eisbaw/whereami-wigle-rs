---
id: TASK-0262
title: 'Decide the fate of the antipodal-fallback guard/field: near-dead in practice'
status: To Do
assignee: []
created_date: '2026-07-08 21:36'
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
