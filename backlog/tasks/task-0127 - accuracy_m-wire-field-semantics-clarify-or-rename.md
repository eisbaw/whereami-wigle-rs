---
id: TASK-0127
title: 'accuracy_m wire field semantics: clarify or rename'
status: To Do
assignee: []
created_date: '2026-05-20 21:19'
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
- [ ] #1 Choose option A or B and document the choice:
- [ ] #2 A) Keep wire name 'accuracy_m' but doc-comment explicitly says: 'weighted spread of inputs around centroid, clamped to [10, 1000]; NOT a calibrated confidence radius'
- [ ] #3 B) Rename wire field 'position_spread_m'; bump protocol version (v=2); client adapts; CHANGELOG note
- [ ] #4 Either option: split the antipodal-fallback case into a separate field (fallback: bool OR accuracy_band enum) so 1000.0 sentinel is no longer in-band
- [ ] #5 Test: antipodal-input locate returns fallback=true with accuracy_m=1000; legitimate ±900m fix returns fallback=false with accuracy_m=900
<!-- AC:END -->
