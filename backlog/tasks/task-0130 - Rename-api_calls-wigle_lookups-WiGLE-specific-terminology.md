---
id: TASK-0130
title: Rename api_calls -> wigle_lookups (WiGLE-specific terminology)
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:20'
updated_date: '2026-07-09 01:19'
labels:
  - docs
  - wigle
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Compass-domain finding #9. db.rs:670 try_reserve_api_call / refund_api_call. Generic 'api_call' name doesn't say which provider's quota. Only WiGLE actually has one (Apple is unmetered). config.rs:60 --daily-limit is also generic. The counter is implicitly WiGLE-specific but named globally. Future second metered provider would need a (provider, counter) schema.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Rename metadata key api_calls_today -> wigle_lookups_today (migration v6 renames the key, preserves the value)
- [x] #2 Rename functions try_reserve_api_call -> try_reserve_wigle_lookup, refund_api_call -> refund_wigle_lookup
- [x] #3 CLI flag: --daily-limit -> --wigle-daily-limit (--daily-limit accepted as alias for one release with a deprecation warning)
- [x] #4 Stats response field api_calls_today -> wigle_lookups_today (wire-compat: emit BOTH for one release)
- [x] #5 Doc-comment notes: we mirror WiGLE's server-side counter optimistically; drift possible if other clients share credentials
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. protocol: add wigle_lookups_today (serde default) beside kept api_calls_today (follow-up ref comment)
2. db.rs: SCHEMA_VERSION 7->8; add migrate_v7_to_v8 (UPDATE metadata SET key rename, lossless); register in ladder
3. db.rs: rename fns (try_reserve/refund/counter/parse) + SQL literals api_calls_today->wigle_lookups_today; DbStats field
4. config.rs: rename flag daily_limit->wigle_daily_limit with alias --daily-limit + testable daily_limit_alias_used + deprecation eprintln
5. server/provider/main/client: update call sites; dual-emit both wire fields same value; client renders new field
6. tests: update all literals; extend cascade builder to v7 + add v7->v8 lossless test; wire fixtures; docs/protocol.md + README
7. file AC#4 follow-up task; nix develop -c just e2e + cargo fmt --check
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented rename api_calls -> wigle_lookups across daemon/protocol/client + docs.

- AC#1: SCHEMA_VERSION 7->8; migrate_v7_to_v8 does `UPDATE metadata SET key='wigle_lookups_today' WHERE key='api_calls_today'` inside run_migration (lossless key rename, preserves value; no-op if absent). Registered in ladder. Proven by two file-backed cascade tests (v7 file with counter=23 opens to v8 with wigle_lookups_today=23, no stale old key) + lockstep step (value 17 survives).
- AC#2: renamed try_reserve_api_call(+_inner/_observing), refund_api_call, api_calls_today(), api_calls_today_for_stats, parse_api_calls_value (+ struct CorruptApiCallsValue and async reserve_api_call family) to wigle_lookup*/wigle_lookups*. SQL literals api_calls_today->wigle_lookups_today (api_calls_date intentionally NOT renamed - internal, out of scope; noted in migration doc).
- AC#3: flag daily_limit -> wigle_daily_limit with clap alias=daily-limit. Deprecation detection via testable config::daily_limit_alias_used(argv) scanning for --daily-limit / --daily-limit=; warn_if_deprecated_daily_limit() eprintln at startup in main.
- AC#4: added wigle_lookups_today to StatsResponse, KEPT api_calls_today; daemon dual-emits SAME value (server.rs). Both serde(default). Client renders new field. Follow-up task-0267 filed + referenced in code comments.
- AC#5: doc-comments with WiGLE-mirror/drift caveat on counter fns + config flag; docs/protocol.md updated for new field + transitional dual-emit.

Gotcha: one existing test (task0245_v7_shape_repair...) stamped v7 then migrate() and asserted ==7; now correctly cascades to v8, changed to SCHEMA_VERSION.

Gate: nix develop -c just e2e PASS (whereamid lib 211, integration 471, protocol wire 40, client 34, config-in-lib covered, shutdown 4, etc; 0 failures) + clippy -D warnings clean; cargo fmt --check clean.

## Review gate (orchestrator, independent) — GO/GO + hardening
- QA (re-run 2x): 812/812 base commit, zero flake; clippy 0 -D warnings; fmt clean. All named migration/alias/dual-emit tests present+passing; rename COMPLETE (grep: only the intentional dual-emit wire field + the migration WHERE clause read the old key; 0 leftover fn/SQL-helper refs). QA caught a harmless self-report label transpose (wire=34, client lib=35, not both 34).
- Architecture: all 6 invariants CONFIRMED with evidence — migration is ATOMIC (single run_migration transaction does rename+version-stamp; no crash window), lossless (non-zero 23 survives real open cascade under new key), re-run-safe (ladder guard + zero-row no-op), fail-loud on collision (metadata.key PK), quota logic byte-identical, dual-emit back-compat both ways, CLI alias detection correct (space + = forms, no false-positive on --wigle-daily-limit, warn once).
- HONESTY CORRECTION (reviewer): the both-keys state is reachable not ONLY by manual corruption but also by a downgrade->upgrade flip (v8 writes wigle_lookups_today; a downgraded old binary writes api_calls_today). Safety UNAFFECTED — the ladder guard prevents migrate_v7_to_v8 from ever re-running on a v8 DB, so the collision path needs version<8 AND both keys. The code doc-comment does NOT overclaim (only the report did).
- HARDENING in-thread (commit 161e0c3): added open_v7_shape_with_both_counter_keys_fails_loud_not_silent — the fail-loud property was prose-only; now a regression test seeds both keys on a v7 file and asserts Database::open errs. Full gate re-run green (813 total).
- LOW follow-ups: task-0268 (api_counter_corruption_denials still generic-prefixed). LOW-3 (transitional UX: new client + pre-rename daemon renders 0 for WiGLE count) documented+accepted.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Renamed the WiGLE daily-quota vocabulary from generic api_calls to provider-specific wigle_lookups across DB, config, wire, and docs. Only WiGLE is metered (Apple is unmetered), so the generic name mis-described a WiGLE-specific counter.

What changed:
- Schema migration v7->v8 (SCHEMA_VERSION 7->8): renames metadata key api_calls_today -> wigle_lookups_today in one atomic transaction (rename + version stamp), lossless (live same-day counter survives the upgrade), no-op when absent, re-run-safe. Fails loud on a key collision rather than silently mis-counting quota.
- Renamed fns/types/SQL: try_reserve_api_call(+inner/observing/async) -> try_reserve_wigle_lookup, refund_api_call -> refund_wigle_lookup, api_calls_today() -> wigle_lookups_today(), parse_api_calls_value -> parse_wigle_lookups_value, CorruptApiCallsValue -> CorruptWigleLookupsValue, DbStats field, all metadata-key SQL literals. Quota reserve/refund/RAII semantics unchanged.
- CLI: --daily-limit -> --wigle-daily-limit; old flag accepted as a deprecated alias (clap alias) with a one-line stderr deprecation warning emitted exactly once (detected via daily_limit_alias_used scanning argv for the old spelling, space and = forms).
- Stats wire: added wigle_lookups_today, KEPT api_calls_today (both serde default, same value) for one transitional release; client renders the new field. Follow-up task-0267 drops api_calls_today next release (referenced in code comments).
- Doc-comments note we optimistically mirror WiGLE server-side counter; drift possible if credentials are shared. protocol.md + README updated.

User impact: transparent upgrade (counter preserved); --daily-limit still works with a deprecation notice; stats JSON carries both counter names transitionally. One temporary mixed-pairing gap: a new client against a pre-rename daemon renders 0 for the WiGLE count (serde default) — documented/accepted.

Tests: 813 workspace (added lossless cascade, absent-key no-op, both-keys fail-loud collision, 3 alias tests, dual-emit + legacy back-compat wire tests); two independent gate runs zero flake; clippy 0; fmt clean.

Not renamed (out of scope, disclosed): api_calls_date (internal reset-date key, off-wire) and internal daily_limit value params (cosmetic). Follow-ups: task-0267 (drop transitional field), task-0268 (api_counter_corruption_denials vocabulary).

Commits: 06e24b4 (rename+migration) + 161e0c3 (collision fail-loud test).
<!-- SECTION:FINAL_SUMMARY:END -->
