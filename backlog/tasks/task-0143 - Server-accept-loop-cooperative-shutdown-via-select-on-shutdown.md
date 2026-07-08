---
id: TASK-0143
title: 'Server accept loop: cooperative shutdown via select! on shutdown'
status: To Do
assignee: []
created_date: '2026-05-20 21:26'
updated_date: '2026-07-08 20:25'
labels:
  - robustness
  - shutdown
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/server.rs:316-332 + src/main.rs:192-205. main.rs calls server_handle.abort() before drain. The accept loop has no shutdown branch — it relies on abort. Forced cancellation can interrupt a connection mid-write; a tokio::select! with shutdown.notified() would let the loop exit at the next accept boundary cleanly.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Server accept loop wraps the accept in tokio::select! { _ = listener.accept() => {...}, _ = state.shutdown.notified() => break }
- [ ] #2 main.rs removes server_handle.abort(); the shutdown notify propagates cleanly to all loops including server
- [ ] #3 5s drain budget (matches task-0075 for other loops) covers server exit too
- [ ] #4 Test: spawn the server, send SIGTERM (or notify), assert it exits within 5s without abort
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Triaged 2026-07-08 (NOT started — deferred to fresh context). This task (2026-05-20) PREDATES the entire shutdown rework (cycles 6/9/11: Notify->tokio_util CancellationToken with biased selects in the named loops; run_shutdown_drain force-aborts survivors via pre-extracted AbortHandles; task-0102 orphan accounting; task-0237 SIGTERM-handler-timing; task-0241 server-task-panic classification). Two problems with taking it as-written now:
1. PREMISE likely OUTDATED: AC rationale says "forced cancellation can interrupt a connection mid-write". But connections are now spawned as SEPARATE tasks (handle_connection; the connection-spawn exemption is task-0201), so aborting the ACCEPT loop only stops accepting NEW connections — in-flight connections are their own tasks drained separately. Re-verify whether the mid-write harm still exists before implementing.
2. Entanglement: the accept loop (server.rs ~1170-1185) pre-acquires a connection_slots semaphore permit BEFORE the accept select!; a cooperative shutdown.cancelled() branch must release that permit cleanly and coordinate with run_shutdown_drain's AbortHandle handling + the biased-select convention used elsewhere. AC#2 (remove server_handle.abort() from main.rs) directly touches the run_shutdown_drain / server-handle-result classification that task-0241 built. This is a DEEP shutdown-architecture change with high regression risk across the drain path — wants a fresh, non-fatigued context and the FULL dual review gate, and should start by re-deriving whether cooperative-break is actually an improvement over the current force-abort-of-accept-loop-only design (which may already be correct). Cross-ref cycles 6/9/11 + tasks 0102/0195/0201/0237/0241.
<!-- SECTION:NOTES:END -->
