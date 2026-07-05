---
id: TASK-0095
title: 'history: expose raw fixes (not only stay-point segments)'
status: Done
assignee:
  - task0095-implementer
created_date: '2026-05-19 06:52'
updated_date: '2026-07-05 14:15'
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
- [x] #1 Add --raw flag and --max-fixes <N> (default 1000) to whereami history; mutually compatible with --from/--to and the relative-range positional
- [x] #2 JSON shape with --raw (snake_case, additive to existing): { ok, v, range: { from_rfc3339, to_rfc3339 }, fixes: [ { at_rfc3339, lat, lon, accuracy_m, n_sources } ... ], elided: Option<i64> }. elided populated only when capped
- [x] #3 Human renderer for --raw: one line per fix, ascending by timestamp, trailing field is '+<distance> / +<elapsed>' where distance is haversine_m to previous shown fix (formatted as m or km depending on magnitude) and elapsed is the time delta (formatted as Ns, Nm, Nh as fits); first row trailing field is '+0.0m / -'
- [x] #4 When capped: render '... N earlier fixes elided; pass --max-fixes <N> or narrow the range' as the first line; show only the most recent 1000 (or --max-fixes value) fixes
- [x] #5 Document --raw, --max-fixes, and the existing min_duration_secs / segment_distance_m thresholds in 'whereami history --help'
- [x] #6 New test: 4 fixes spanning 2 min at same lat/lon -> --raw returns all 4 with sensible distance/elapsed fields; without --raw returns 0 segments (existing behavior)
- [x] #7 New test: 1500 fixes -> --raw without --max-fixes returns 1000 fixes + elided=500
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Extend the opt-in history request and client/CLI surfaces without changing legacy history() or history_raw() bytes: add raw=true plus max_fixes, separate raw-fix response/types/APIs, and reject successful legacy segment responses to a raw-fix request.
2. Add a bounded guarded database query: within one deferred read snapshot, count the inclusive range, select only the newest N by timestamp/id, reverse to ascending output, and return exact elision while preserving the existing unbounded segmented query unchanged.
3. Branch raw versus segmented work only after shared range validation and single-flight admission; keep guarded reader cancellation, spawn_blocking serialization, privacy ticket, and response-held lease identical through socket write.
4. Implement the exact raw human renderer, CLI help, protocol/README docs, and real dispatch tests for four short fixes versus zero segments and 1500 rows versus the default cap.
5. Run focused/mutation tests, formatting/lint/full e2e, freeze the diff, pass parallel QA/architecture review, close the tracker, and commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Phase 3 pre-implementation audit decisions: legacy segmented HistoryResponse and public history()/history_raw() remain unchanged; add history_fixes()/history_fixes_raw() because _raw already means unparsed transport JSON. Opt-in wire request is raw=true with max_fixes; max without raw is an error. Daemon owns default 1000 and validates 1..=10000, where 10000 is a protocol/DoS safety ceiling while 1000 remains the overrideable soft default. A correct new raw response is {ok,v,range:{from_rfc3339,to_rfc3339},fixes:[...],elided?}; range is normalized inclusive UTC bounds, legacy response stays {from,to,segments}, and elided is omitted unless total>limit. Both typed and byte-preserving JSON raw-fix methods validate that ok=true includes the new raw shape so an old daemon ignoring request fields fails loudly; correct new-daemon JSON bytes remain unchanged. DB count and newest-page selection share one deferred transaction snapshot; select DESC by (at_rfc3339,id), retain at most N, then reverse to ASC with id tie-break. Exact count can scan the index but memory/response allocation is O(N); never load all then truncate and never preallocate from untrusted input. Raw and segmented share one history semaphore, guarded interruptible reader, blocking serialization/cancellation, privacy, and response-write lease. Human raw output has no segment header; ASCII capped banner per AC4; first shown row +0.0m / - even after elision; distance one decimal m below 1000 and km at/above; elapsed floored whole units with 60s=>1m and 3600s=>1h; duplicate timestamps +0s; invalid timestamp renders ? without panic. Use six coordinate decimals for consistency with locate. --max-fixes without --raw and zero/above-ceiling fail at Clap; raw remains compatible with relative or from/to ranges. No Nix change.

Completed after mandatory parallel QA and mped-architect review. Initial architecture NO-GO identified incomplete incident-renderer coverage, signed sub-second elapsed handling, and redundant raw-response allocation; all three were fixed at root cause and both re-review gates returned GO on code hash e5046280e5998af0e4517fce527a655bd7103d560d491f6a137c4e6aafb2d94a. Final reviewer run: 622 tests, strict Clippy, formatting, diff checks, repeated renderer/cap/snapshot checks all green. Honest limitation: raw memory is O(max_fixes), but exact indexed COUNT remains O(total matching rows); legacy segmented history remains intentionally unbounded for compatibility.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added opt-in bounded raw history across daemon, database, client API, CLI, help, and protocol docs. Raw mode returns the newest capped fixes in stable ascending order with exact elision, preserves legacy history bytes/schema, rejects old-daemon false successes, and renders fix-to-fix distance/elapsed output. Database count/page selection shares one snapshot; raw and segmented paths share admission, cancellation, privacy, blocking serialization, and response-write lease.
<!-- SECTION:FINAL_SUMMARY:END -->
