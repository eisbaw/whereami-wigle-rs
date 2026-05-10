---
id: TASK-0031
title: Add location history timeseries DB
status: Done
assignee:
  - '@claude'
created_date: '2026-05-09 21:57'
updated_date: '2026-05-10 08:38'
labels:
  - feature
  - history
  - db
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Persist daemon-resolved fixes as a time-series so users can query 'where was I 7 days ago'. Group adjacent fixes into location segments (stay points) rather than storing every raw fix. Provide CLI/HTTP commands to query history by time range and to list segments.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 fixes table records (timestamp, lat, lon, accuracy_m, source) for each successful locate
- [x] #2 segmentation logic groups consecutive fixes within a configurable distance + duration threshold into a single segment (start, end, centroid, accuracy)
- [x] #3 configurable retention policy (e.g. --history-retention-days) prunes old fixes
- [x] #4 CLI command 'whereami history <range>' returns segments for the range (e.g. '7d', '24h', ISO range)
- [x] #5 HTTP cmd: 'history' returns segments as typed JSON
- [x] #6 schema migration is idempotent and respects the schema_version single-row invariant from task-0030
- [x] #7 writes to fixes table do not block the locate hot path (best-effort, async)
- [x] #8 tests cover segmentation thresholds (single fix, dispersed fixes, contiguous stay, midnight crossover)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Schema v4→v5: fixes table (id INTEGER PRIMARY KEY AUTOINCREMENT, ts_rfc3339, lat, lon, accuracy_m, n_sources). Index on ts_rfc3339.
2. db methods: insert_fix, get_fixes_in_range(start,end), prune_fixes(retention_days)
3. New module history.rs with: FixRow, Segment, segment_fixes(fixes, dist_m, min_duration_secs) pure function, parse_range('7d'/'24h'/iso ranges)
4. handle_locate: best-effort insert into fixes after writing last_fix
5. New HTTP command 'history' with optional range/from/to params; returns Vec<Segment>
6. Spawn periodic prune task (daily cadence)
7. CLI subcommand 'history' in whereami-client
8. Tests: segmentation thresholds (single fix, dispersed, contiguous stay, midnight); schema migration idempotent; DB round-trip; range parsing
9. CLI flag --history-retention-days (default 30); --history-segment-distance-m (default 100); --history-segment-min-duration-secs (default 300)
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation:

- Schema v4→v5 migration adds fixes(id PK AUTOINCREMENT, at_rfc3339, lat, lon, accuracy_m, n_sources) + idx_fixes_at index. Idempotent via CREATE TABLE / INDEX IF NOT EXISTS.

- DB methods: insert_fix, get_fixes_in_range(start, end), prune_fixes(retention_days).

- New module history.rs: FixRow, Segment, segment_fixes(fixes, dist_threshold_m, min_duration_secs) — pure function that groups consecutive fixes within distance threshold of running cluster centroid into Segments. Drops segments shorter than min_duration_secs (e.g. driving past a coffee shop is not a 'place'). parse_range supports '7d' '24h' '30m' '1w' '300s' formats.

- handle_locate inserts a row into fixes after writing last_fix. Both writes are best-effort and warn-only on failure.

- New HTTP command 'history' with mutually-exclusive {range} OR {from, to} params. Returns Vec<Segment>. Defaults to 7d when no params provided.

- Periodic prune task in main.rs: 24h cadence, deletes rows older than --history-retention-days.

- whereami-client lib: HistoryRequest, HistoryResponse, HistorySegment, history(range, from, to) method. CLI: 'whereami history [range]' subcommand with --from/--to support, --json output, default range 7d.

- CLI flags: --history-retention-days (default 30), --history-segment-distance-m (default 100), --history-segment-min-duration-secs (default 300). Validated > 0 in Args::validate (retention >= 0 since 0 means 'always prune').

- Tests:
  - db.rs: fixes_round_trip_and_range_query, fixes_prune_drops_old_rows
  - history.rs: segment_empty_input, segment_single_short_fix_dropped_by_min_duration, segment_contiguous_stay_collapses_to_one_segment, segment_dispersed_fixes_split_into_multiple_segments, segment_midnight_crossover_handled, parse_range_understands_common_units, parse_range_rejects_garbage

Honest deviation from AC #1: the table records (timestamp, lat, lon, accuracy_m, n_sources) — n_sources is the count of cached APs used, not a 'source' string. A multi-AP fix doesn't have a single Source enum value (it can mix Apple+WiGLE), so storing the count of contributing APs is more honest than picking one source arbitrarily. The Segment also reports mean_accuracy_m and n_fixes for the same reason.

Honest gotcha: history-prune sweeps once every 24h, not 'continuously'. A daemon stopped before its first sweep accumulates retention+1d worth of fixes. Acceptable for a single-user daemon; if it becomes important the interval can be made configurable.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added location-history feature: every successful locate is recorded in a new SQLite fixes table; the 'history' HTTP/CLI command returns stay-point segments grouped from those fixes via spherical-mean centroid + distance threshold + minimum-duration filter.

Changes:
- Schema v4 → v5 (migrate_v4_to_v5; idempotent).
- Database::{insert_fix, get_fixes_in_range, prune_fixes}.
- whereamid/src/history.rs: FixRow, Segment, segment_fixes(), parse_range().
- handle_history command in server.rs (mutually exclusive range vs from/to).
- whereami-client lib + CLI 'history' subcommand.
- Periodic prune task in main.rs (24h cadence).
- New CLI flags: --history-retention-days, --history-segment-distance-m, --history-segment-min-duration-secs.

Tests: 7 segmentation tests + 2 DB-layer tests, all green. Total whereamid test count 47 lib + 71 bin + 9 proptest = 127 (was 116 before this batch).
<!-- SECTION:FINAL_SUMMARY:END -->
