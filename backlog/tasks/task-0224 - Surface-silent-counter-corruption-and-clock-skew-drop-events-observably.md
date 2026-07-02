---
id: TASK-0224
title: Surface silent counter-corruption and clock-skew-drop events observably
status: To Do
assignee: []
created_date: '2026-07-02 05:27'
updated_date: '2026-07-02 22:43'
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
- [ ] #1 when set_last_fix drops a write because the incoming timestamp is older, a debug!/warn! is emitted naming both timestamps (skew visibility)
- [ ] #2 counter-corruption is distinguishable from normal quota exhaustion at the provider or stats layer (not only via an internal warn)
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from task-0102: a new observability counter orphan_tasks_aborted (AtomicU64 on DaemonState) is now surfaced in StatsResponse + handle_stats and documented in docs/protocol.md, following the db_write_failures pattern. Use the same pattern for the counter-corruption / clock-skew-drop events this task tracks: AtomicU64 field -> load(Relaxed) in handle_stats -> StatsResponse field -> protocol.md bullet.

forward-carried from task-0102 review fix (bd0cb9f): the persist-at-shutdown/reload-at-startup pattern for counters now exists — metadata key orphans_aborted_last_shutdown + db::set_metadata/get_metadata + load_orphan_count in main.rs. If 0224 surfaces counter-corruption/clock-skew events via stats, reuse this pattern; an in-memory-only AtomicU64 bumped near shutdown is unobservable (that was exactly the 0102 AC#3 defect).
<!-- SECTION:NOTES:END -->
