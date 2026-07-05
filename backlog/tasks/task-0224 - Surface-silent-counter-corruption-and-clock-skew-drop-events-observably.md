---
id: TASK-0224
title: Surface silent counter-corruption and clock-skew-drop events observably
status: Done
assignee: []
created_date: '2026-07-02 05:27'
updated_date: '2026-07-05 08:26'
labels:
  - correctness
  - observability
dependencies:
  - TASK-0097
  - TASK-0100
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Two P3 observability gaps surfaced during the task-0097/0100 review gate: (1) after fail-closed (task-0097), a corrupt api_calls_today row makes try_reserve_api_call return Ok(false), which the provider maps to Skipped - indistinguishable at the provider/status layer from normal quota exhaustion; the only signal is an internal db.rs warn. (2) The last_fix monotonicity guard (task-0100) silently drops an older write (execute returns 0 rows) with no log, so a real clock-skew/NTP-step event is invisible. Ref whereamid/src/db.rs (try_reserve_api_call, set_last_fix), server.rs:986.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 when set_last_fix drops a write because the incoming timestamp is older, a debug!/warn! is emitted naming both timestamps (skew visibility)
- [x] #2 counter-corruption is distinguishable from normal quota exhaustion at the provider or stats layer (not only via an internal warn)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Replace the bool quota reservation result with typed Reserved, Exhausted, and CounterCorrupt decisions while preserving cancellation-owned QuotaLease semantics. Increment one accurately named since-start api_counter_corruption_denials AtomicU64 inside the DB actor closure exactly when CounterCorrupt is produced; keep corruption and normal exhaustion mapped to ProviderOutcome::Skipped per TASK-0097/0189, but map genuine SQLite/actor errors to hard-transient and record db_write_failures. 2. Make api counter parsing side-effect-free and emit contextual reservation/refund/stats warnings instead of a misleading reservation warning from every caller. 3. Centralize the duplicated last_fix UPSERT behind an Applied/DroppedOlder outcome shared by set_last_fix and persist_fix_and_history. Commit first, then warn once at default-visible level with escaped incoming and stored timestamps; the combined path must still append history and rollback must emit nothing. 4. Add the corruption-denial field additively to daemon stats, typed client with serde default, human CLI, protocol and changelog; document reset on daemon start and keep wire v1. Do not add a skew counter. 5. Add deterministic DB actor cancellation, quota-decision/provider-mapping, DB-error, stats compatibility/rendering, direct/combined skew, equal/newer, and rollback tests. Run focused stress and nix-shell just e2e, require parallel QA/architecture GO, complete tracker, rerun E2E, and commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from task-0102: a new observability counter orphan_tasks_aborted (AtomicU64 on DaemonState) is now surfaced in StatsResponse + handle_stats and documented in docs/protocol.md, following the db_write_failures pattern. Use the same pattern for the counter-corruption / clock-skew-drop events this task tracks: AtomicU64 field -> load(Relaxed) in handle_stats -> StatsResponse field -> protocol.md bullet.

forward-carried from task-0102 review fix (bd0cb9f): the persist-at-shutdown/reload-at-startup pattern for counters now exists — metadata key orphans_aborted_last_shutdown + db::set_metadata/get_metadata + load_orphan_count in main.rs. If 0224 surfaces counter-corruption/clock-skew events via stats, reuse this pattern; an in-memory-only AtomicU64 bumped near shutdown is unobservable (that was exactly the 0102 AC#3 defect).

Preimplementation adjudication (2026-07-05): the prior persistence note is superseded for this task. Orphan aborts require shutdown persistence because they are produced after the socket closes; quota-denial and skew events occur while stats/logs are live. Persisting a shutdown snapshot would mix prior-run data into a current-run name and still lose crashes, so api_counter_corruption_denials is explicitly cumulative since daemon start and resets on restart. AC1 requires a timestamp-rich warning, not a skew counter. TASK-0097 AC2 explicitly requires corrupt quota to map to Skipped, and TASK-0189 deliberately treats Skipped as abstention; TASK-0224 adds observability but does not reverse that accepted resolution policy. Genuine reservation DB errors were never an accepted deterministic skip and become hard-transient plus db_write_failures. Instrument both set_last_fix and the production persist_fix_and_history duplicate; warnings describe rejection of the persisted last_fix update because memory/history may still accept the causally newer observation. The live-memory versus wall-clock-newer durable last_fix divergence is a separate correctness policy, not changed here.

Final review verification (2026-07-05): corruption decisions count exactly at actor execution; queued cancellation counts zero and post-decision waiter loss counts once. Both last_fix paths share one transactional helper and warn only after commit; rollback emits no phantom warning. Parallel QA and architecture returned GO with no findings on frozen hash f0c041ce after 491-test E2E and 340 focused stress executions.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Introduced typed WiGLE quota decisions so exhausted and corrupt counters are distinguishable without changing accepted Skipped policy. A new additive api_counter_corruption_denials stat counts executed corrupt-counter reservation denials since daemon start, with actor-linearized cancellation behavior; genuine reservation DB errors now hard-stop and increment db_write_failures. Made counter parsing side-effect-free with caller-specific escaped warnings. Centralized both last_fix UPSERT paths and now warn after successful commit with incoming and stored timestamps when the persisted update is older; combined history still commits and rollback emits nothing. Updated daemon/client/CLI/protocol v1 compatibility and changelog. Verified 491-test E2E, strict Clippy, formatting, 340 focused stress executions, and final parallel QA/architecture GO with no findings.
<!-- SECTION:FINAL_SUMMARY:END -->
