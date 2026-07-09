---
id: TASK-0268
title: Rename api_counter_corruption_denials wire field to wigle_ vocabulary
status: Done
assignee:
  - '@myself'
created_date: '2026-07-09 01:18'
updated_date: '2026-07-09 08:07'
labels:
  - refactor
  - wigle
  - wire-format
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
task-0130 review LOW: the daily-quota vocabulary moved to wigle_ (wigle_lookups_today), but StatsResponse.api_counter_corruption_denials keeps the generic api_ prefix. It is a distinct concept (in-memory count of WiGLE reservations denied because the persisted same-day counter was malformed, not the daily quota itself), but if the goal is fully WiGLE-specific wire vocabulary it should become e.g. wigle_counter_corruption_denials. Wire-compat: emit both for one release, same as task-0130 did for the counter, then drop via a task like task-0267.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Decision recorded: rename to wigle_-prefixed name OR keep generic with a justifying doc-comment
- [x] #2 If renamed: StatsResponse emits both old+new for one transitional release (serde default both), follow-up filed to drop the old field
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Decision (AC#1): RENAME the wire field to wigle_counter_corruption_denials (WiGLE-specific concept; consistent with task-0130 vocab move). Record rationale in notes.
2. AC#2 dual-emit mirroring task-0130:
   - protocol StatsResponse: add wigle_counter_corruption_denials (#[serde(default)], doc); re-doc api_counter_corruption_denials as DEPRECATED transitional mirror -> task-0275.
   - whereamid/server.rs: fill BOTH wire fields from the SAME api_counter_corruption_denials AtomicU64 load. Keep internal Arc name (wire is what AC#2 requires).
   - client: render the NEW field; update fixture.
   - wire.rs: STATS_FIXTURE + round-trip carry both; legacy fixture without new field decodes to 0.
   - server stats test: assert both wire fields.
   - docs/protocol.md: document new field + deprecated mirror.
3. Follow-up task-0275 filed to drop api_counter_corruption_denials post-transition; referenced in code comment.
4. Gate: nix develop -c just e2e + cargo fmt --check.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Decision (AC#1): RENAMED the wire field to wigle_counter_corruption_denials. Rationale: the field counts WiGLE reservations denied due to a malformed same-day WiGLE counter — WiGLE-specific, so it should follow the wigle_ vocabulary established by task-0130, not the generic api_ prefix.

AC#2 (transitional dual-emit, mirrors task-0130):
- whereami-protocol StatsResponse: added wigle_counter_corruption_denials (#[serde(default)], doc); re-doc api_counter_corruption_denials as DEPRECATED transitional mirror -> task-0275.
- whereamid/server.rs: both wire fields filled from the SAME api_counter_corruption_denials AtomicU64 load (same value). Internal Arc name KEPT (only the wire changed).
- client renders the new wigle_ field.
- wire.rs: STATS_FIXTURE + round-trip carry both; dual-emit decode asserts both=3; legacy fixture (neither field) serde-defaults both to 0.
- server stats test asserts both wire fields=3.
- docs/protocol.md documents new field + deprecated mirror.

Follow-up filed: task-0275 (drop api_counter_corruption_denials post-transition), referenced in code comments at the deprecated field/dual-emit site.

Gate: nix develop -c just e2e PASS (900 tests, clippy -D warnings clean); cargo fmt --check clean. Commit fad2f74 (code+protocol.md only, no backlog). Left In Progress for review; ACs not checked.

## Review gate (orchestrator in-thread gate + independent correctness review) — GO/GO
- Gate (orchestrator in-thread): diff is wire+client+protocol.md+tests only (5 files); 900/900; clippy 0 -D warnings; fmt clean.
- Independent review: all 5 CONFIRMED. (1) Both wire fields filled from ONE .load(Relaxed) (server.rs:2962-2964 -> 3000 wigle_ = the local, 3005 api_ = field-init shorthand of the same local): identical value, one source; exact mirror of the api_calls_today/wigle_lookups_today dual-emit above it. (2) Back-compat both ways: both #[serde(default)]; wire tests assert dual-emit both==3 + legacy_stats_defaults_missing_counters_to_zero (neither field -> both 0). (3) Counting UNCHANGED: the fetch_add(1) at db.rs:1014 is NOT in the diff; AtomicU64 + internal Arc name unchanged. (4) Client render minimal (main.rs:453 reads wigle_); deprecation documented (drop-follow-up task-0275 referenced in lib.rs + server.rs:3003 + protocol.md). (5) Protocol stays v=1.
- LOW (cosmetic, accepted, deferred to task-0275): the human display LABEL still reads "API counter corruption denials" while reading the wigle_ field — consistent with the existing task-0130 precedent ("API today:" reads wigle_lookups_today); the transitional labels get revisited when task-0275 drops the deprecated api_ fields.
- Follow-up filed: task-0275 (drop api_counter_corruption_denials from the wire next release), mirroring task-0267 for api_calls_today.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Renamed the StatsResponse api_counter_corruption_denials wire field to wigle_counter_corruption_denials, extending the task-0130 WiGLE-vocabulary move to the corruption-denial counter (which is WiGLE-specific — it counts reservations denied because the persisted same-day WiGLE counter was malformed).

Transitional dual-emit (identical to task-0130): the daemon emits BOTH wigle_counter_corruption_denials (new) and api_counter_corruption_denials (deprecated mirror), filled from the same AtomicU64 load, both #[serde(default)]. Old client reads api_ (still present); new client reads wigle_. Back-compat both directions; protocol stays v=1. The corruption-denial counting logic is untouched (only the wire emission + client render changed). Internal Arc field name kept (wire-only change, lower risk).

Follow-up task-0275 filed to drop the deprecated api_counter_corruption_denials field next release (mirroring task-0267), referenced in code + protocol.md.

Tests: dual-emit round-trip (both == same value) + legacy-fixture back-compat (neither field -> both 0) + server stats assertion. 900 workspace tests, dual gate GO/GO. Commit fad2f74.

Accepted cosmetic (deferred to task-0275): the human display label still reads "API..." during the transition (same as task-0130 left "API today:").
<!-- SECTION:FINAL_SUMMARY:END -->
