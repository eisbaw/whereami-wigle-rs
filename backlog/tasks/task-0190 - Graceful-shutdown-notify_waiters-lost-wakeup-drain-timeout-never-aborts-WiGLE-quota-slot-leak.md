---
id: TASK-0190
title: >-
  Graceful shutdown: notify_waiters lost-wakeup, drain timeout never aborts,
  WiGLE quota slot leak
status: Done
assignee:
  - '@claude'
created_date: '2026-07-02 04:50'
updated_date: '2026-07-02 22:21'
labels:
  - correctness
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The three background loops register shutdown.notified() only inside their select!. Notify::notify_waiters stores no permit, so a SIGTERM that lands while a loop is mid-iteration (scan 1.5-11s, drain up to 10 BSSIDs x 2 providers x 15s, prune) is lost: the loop re-parks and runs forever, even launching new scans/resolver tasks after shutdown began. main then always eats the full 5s drain timeout whose handler only logs - scan/pending/history handles are never .abort()ed despite the comment. Runtime-drop cancellation can kill drain_once between try_reserve_api_call (committed) and the refund, leaking a persisted quota slot for the day. Ref whereamid/src/main.rs:174-205,262-265, pending.rs:32-40.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 SIGTERM during a scan/drain/prune iteration exits the loop at the next await boundary without waiting out the sleep interval (use CancellationToken or watch)
- [x] #2 no new scan or resolver work is started after the shutdown signal
- [x] #3 the drain timeout path actually aborts remaining tasks; daemon exits within the drain budget in a mid-scan test
- [x] #4 a quota slot reserved when shutdown interrupts a lookup is refunded or never leaks into the persisted counter
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add tokio-util = "0.7" (NO features; CancellationToken is in the ungated pub mod sync — "sync" is NOT a real feature, orchestrator steer corrected).
2. DaemonState: replace shutdown: Notify with shutdown: CancellationToken; add tasks: Mutex<JoinSet<()>> + orphan_tasks_aborted: AtomicU64. Update all 4 constructors (main/pending/resolver/server-dispatch).
3. Loop rework (level-triggered, biased select): scan races wifi_scan vs cancelled() + sleep vs cancelled(); pending races drain_once vs cancelled() + sleep; history sleep vs cancelled(). No new work after cancel.
4. scanner.rs: .kill_on_drop(true) on the 4 nmcli/iw Commands so a dropped scan future does not orphan a child.
5. Quota RAII guard in provider.rs: QuotaSlot armed on reserve; defuse() on Found/NotFound (real upstream spend); Drop refunds on RateLimited/Network/cancel-drop. One refund path. Drop never panics (warn+record_db_failure).
6. main: cancel() -> abort server (Option/flag so no double-poll, 0195) -> run_shutdown_drain(budget, named, joinset) which awaits within budget then force-aborts survivors + returns count -> store in orphan_tasks_aborted.
7. Tests: paused-time drain (5x10s stuck -> orphan==5, returns in budget); guard drop/defuse + abort-mid-await refund; loop wakes-on-cancel; integration tests/shutdown.rs (bind-failure no-panic + SIGTERM <=4s).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented across commits 62d7957 (loops+drain+0195), dcbad2b (quota guard), 414a048 (tracked spawns+stat), 31c0491 (integration tests).

Gotchas / lessons (forward-carry):
- tokio-util has NO "sync" feature. CancellationToken lives in the UNGATED `pub mod sync` (verified in crate source). The orchestrator steer `features = ["sync"]` would fail the build; used `tokio-util = "0.7"` with default features. It was already a transitive dep (reqwest/hyper), so no new crate downloaded.
- tokio "full" does NOT include `test-util`; `#[tokio::test(start_paused=true)]` needs it. Added `tokio = { features=["test-util"] }` under [dev-dependencies] so it never leaks into the release binary.
- run_shutdown_drain relies on edition-2021 disjoint closure capture: the graceful async block MOVES `named` (owned JoinHandles, awaited once) but BORROWS `tracked` (JoinSet) so it survives the timeout for force-abort + count. Verified it compiles.
- Used `biased;` in every loop select so cancellation is checked first (deterministic under paused clock; never runs one extra work iteration when both arms are ready).
- pending loop races drain_once vs cancelled at the LOOP level (did not touch resolve_chain/drain_once internals per steer). Safe to drop mid-pass: per-BSSID DB writes are atomic and the WiGLE quota slot is refunded by its RAII guard on drop.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Replace the lost-wakeup-prone tokio::sync::Notify shutdown with a level-triggered tokio_util CancellationToken, fix the drain that never aborted, and stop the WiGLE quota-slot leak.

What changed:
- DaemonState.shutdown is now a CancellationToken (sticky: once cancel(), cancelled() is always-ready and is_cancelled() true) — kills the lost-wakeup class where a SIGTERM landing mid-iteration was dropped.
- scan/pending/history loops select work AND sleep against cancelled() (biased); scan races wifi_scan itself, and scanner.rs sets kill_on_drop on the nmcli/iw children so a cancelled scan orphans no subprocess.
- main shutdown extracted into run_shutdown_drain(budget): cancel -> abort server -> await named handles + tracked JoinSet within 5s -> force-abort survivors via pre-extracted AbortHandles and count them into orphan_tasks_aborted. No JoinHandle is polled twice (task-0195).
- WiGLE quota: QuotaSlot RAII guard, armed on reserve, defused on Found/authoritative-NotFound, refunded by Drop on RateLimited/Network AND cancellation-drop (single refund path; Drop never panics).

Why: SIGTERM could leave loops running forever and spawning new resolver work, main always ate the full 5s drain, and an interrupted lookup leaked a persisted daily-quota slot.

Tests: paused-clock drain tests (5x10s stuck -> aborted+counted within budget; stuck named handle; only survivors counted); pending loop wake-on-cancel; quota drop/defuse + abort-mid-await refund; integration SIGTERM exit <4s (old code always ate 5s).

Risks/limits: the SIGTERM integration test is timing-based (generous 4s margin; real exit <1s). A drain_once already mid-HTTP when cancel fires is abandoned at its await (safe due to atomic per-BSSID writes + quota guard); resolve_chain itself was intentionally not made cancellation-aware.
<!-- SECTION:FINAL_SUMMARY:END -->
