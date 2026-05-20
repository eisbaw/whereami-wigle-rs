---
id: TASK-0095
title: 'history: expose raw fixes (not only stay-point segments)'
status: To Do
assignee: []
created_date: '2026-05-19 06:52'
updated_date: '2026-05-19 07:07'
labels:
  - ux
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
whereami history applies stay-point segmentation: groups fixes within --history-segment-distance-m (100m default) of each other AND discards any segment shorter than --history-segment-min-duration-secs (300s default). For users with sparse locate invocations or short stops, the resulting view silently drops most of the data. Real incident: user logged 4 fixes at Maglevænget over 2 min 9 sec (below 300s floor); entire visit dropped from 'whereami history 7d', giving the false impression of no movement.

Decision pinned: keep min_duration_secs default at 300. Lowering it would change segmentation semantics for everyone and is a separate UX call (file a follow-up task if desired). The fix here is to expose the raw fixes as an alternative view.

Output cap: --raw without --max-fixes uses 1000 as a soft cap. When the cap is hit, render a summary line ('… N earlier fixes elided; pass --max-fixes <N> or narrow the range') and the most recent 1000.

Human-renderer format for --raw: one line per fix, ascending time:
  2026-05-19T06:42:36+00:00  55.74391, 12.57933  ±90m  8src  +0.0m / +14s
where the trailing field is (fix-to-fix haversine distance / elapsed time since the previous shown fix). First row shows '+0.0m / -' for both fields.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Add --raw flag and --max-fixes <N> (default 1000) to whereami history; mutually compatible with --from/--to and the relative-range positional
- [ ] #2 JSON shape with --raw (snake_case, additive to existing): { ok, v, range: { from_rfc3339, to_rfc3339 }, fixes: [ { at_rfc3339, lat, lon, accuracy_m, n_sources } ... ], elided: Option<i64> }. elided populated only when capped
- [ ] #3 Human renderer for --raw: one line per fix, ascending by timestamp, trailing field is '+<distance> / +<elapsed>' where distance is haversine_m to previous shown fix (formatted as m or km depending on magnitude) and elapsed is the time delta (formatted as Ns, Nm, Nh as fits); first row trailing field is '+0.0m / -'
- [ ] #4 When capped: render '... N earlier fixes elided; pass --max-fixes <N> or narrow the range' as the first line; show only the most recent 1000 (or --max-fixes value) fixes
- [ ] #5 Document --raw, --max-fixes, and the existing min_duration_secs / segment_distance_m thresholds in 'whereami history --help'
- [ ] #6 New test: 4 fixes spanning 2 min at same lat/lon -> --raw returns all 4 with sensible distance/elapsed fields; without --raw returns 0 segments (existing behavior)
- [ ] #7 New test: 1500 fixes -> --raw without --max-fixes returns 1000 fixes + elided=500
<!-- AC:END -->
