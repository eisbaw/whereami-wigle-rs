---
id: TASK-0143
title: 'Server accept loop: cooperative shutdown via select! on shutdown'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:26'
updated_date: '2026-07-09 08:55'
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
- [x] #1 Server accept loop wraps the accept in tokio::select! { _ = listener.accept() => {...}, _ = state.shutdown.notified() => break }
- [x] #2 main.rs removes server_handle.abort(); the shutdown notify propagates cleanly to all loops including server
- [x] #3 5s drain budget (matches task-0075 for other loops) covers server exit too
- [x] #4 Test: spawn the server, send SIGTERM (or notify), assert it exits within 5s without abort
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Accept loop (run_server_inner): wrap permit acquire in biased select! with state.shutdown.cancelled()=>break; add biased + cancelled()=>break branch to accept select!. Emit info! clean-exit log on break.
2. main.rs: remove server_handle.abort() on signal path; still move handle into bounded drain. Update comment. Leave failure path (task-0195) untouched.
3. Confirm drain force-abort backstop for server preserved (main.rs:671-679).
4. tests/shutdown.rs: extend sigterm_exits_within_drain_budget to assert cooperative clean-exit log line + no abort.
5. Gate: just e2e, cargo fmt --check, shutdown test 3x.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Triaged 2026-07-08 (NOT started — deferred to fresh context). This task (2026-05-20) PREDATES the entire shutdown rework (cycles 6/9/11: Notify->tokio_util CancellationToken with biased selects in the named loops; run_shutdown_drain force-aborts survivors via pre-extracted AbortHandles; task-0102 orphan accounting; task-0237 SIGTERM-handler-timing; task-0241 server-task-panic classification). Two problems with taking it as-written now:
1. PREMISE likely OUTDATED: AC rationale says "forced cancellation can interrupt a connection mid-write". But connections are now spawned as SEPARATE tasks (handle_connection; the connection-spawn exemption is task-0201), so aborting the ACCEPT loop only stops accepting NEW connections — in-flight connections are their own tasks drained separately. Re-verify whether the mid-write harm still exists before implementing.
2. Entanglement: the accept loop (server.rs ~1170-1185) pre-acquires a connection_slots semaphore permit BEFORE the accept select!; a cooperative shutdown.cancelled() branch must release that permit cleanly and coordinate with run_shutdown_drain's AbortHandle handling + the biased-select convention used elsewhere. AC#2 (remove server_handle.abort() from main.rs) directly touches the run_shutdown_drain / server-handle-result classification that task-0241 built. This is a DEEP shutdown-architecture change with high regression risk across the drain path — wants a fresh, non-fatigued context and the FULL dual review gate, and should start by re-deriving whether cooperative-break is actually an improvement over the current force-abort-of-accept-loop-only design (which may already be correct). Cross-ref cycles 6/9/11 + tasks 0102/0195/0201/0237/0241.

Implemented cooperative accept-loop shutdown (commit 9a963c4).

- server.rs run_server_inner: wrapped BOTH per-iteration await points in biased select! against state.shutdown.cancelled() (CancellationToken, not the stale .notified() Notify API from task-0190). Permit acquire and the accept select! each break on cancel. Added run_server clean-exit info! log + Ok(()) return.
- main.rs: removed server_handle.abort() on the signal path; handle still moves into the SAME bounded drain (run_shutdown_drain awaits it, force-abort backstop preserved at main.rs:671-679). Failure path (task-0195) untouched.
- tests/shutdown.rs: extended sigterm_exits_within_drain_budget to assert the cooperative clean-exit log line + that the drain backstop did not fire.

Gate: just e2e = 902 passed / 0 failed (clippy -D warnings clean). cargo fmt --check clean. shutdown suite 3x green, sigterm test <1s each (well under 5s budget). Left In Progress for review.

## Review gate (orchestrator, independent) — GO/GO
- QA (agent 5x + orchestrator in-thread 5x): shutdown suite 4/4 pass every run; sigterm_exits_within_drain_budget ~0.3-0.4s each (wide margin under 5s); the 3 cooperative-exit assertions (clean-exit log present, "did not drain within" force-abort warning NOT fired, <4s) held all runs; 902/902; clippy 0 -D warnings; fmt clean; no flake.
- Architecture: all 5 CONFIRMED. (1) BOTH per-iteration await points (permit acquire_owned + accept select!) open with biased; + _ = state.shutdown.cancelled() => break; no uncovered await (peer_cred/spawn_handler are sync); on break run_server logs + returns Ok. (2) main.rs abort() REMOVED on the signal path; handle still moves into the bounded drain (Some(server_handle)); state.shutdown.cancel() fires before the drain; force-abort backstop preserved (5s timeout -> abort+drop, deliberate per 0241) so SIGTERM stays BOUNDED. (3) failure path (task-0195) UNTOUCHED (diff doesn\x27t touch that arm; no double-poll). (4) ordering preserved (cancel -> accept breaks -> in-flight spawn_tracked drain -> force-abort survivors); connection handling unchanged. (5) test genuinely BITES: the cooperative-exit log is reachable ONLY via the cancel break (an aborted future never reaches it) + the no-force-abort-warning assertion — a fall-back to the abort backstop fails it.
- Cooperative exit GENUINE (not backstop-reliant); SIGTERM bounded even if a park point wedged (5s timeout backstop).
- LOW filed as task-0276: the permit-acquire cancel branch is verified by inspection but not exercised by a test (idle daemon parks on accept, not the semaphore) — a saturated-semaphore SIGTERM test would cover it. LOW (informational): pre-existing 2x5s total shutdown ceiling (main drain + wipe drain sequential), unrelated.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
The server accept loop now shuts down COOPERATIVELY via the task-0190 CancellationToken instead of being force-aborted — a forced abort could interrupt the loop mid-spawn.

- run_server accept loop: BOTH per-iteration await points (the connection-admission permit acquire AND the unix/tcp accept select!) now open with `biased;` and a `_ = state.shutdown.cancelled() => break` branch, so a shutdown while parked at EITHER (incl. parked on the permit with the semaphore exhausted) exits promptly. On break it logs a cooperative-exit line and returns Ok. Connection handling (spawn_tracked) is unchanged.
- main.rs: removed server_handle.abort() on the signal path — state.shutdown.cancel() now propagates to the accept loop\x27s select! for a clean exit; the handle still moves into the SAME bounded drain, which awaits its clean exit with the 5s force-abort backstop preserved (defense-in-depth). The task-0195 failure path is untouched (no double-poll).
- Test: sigterm_exits_within_drain_budget extended to assert the cooperative clean-exit log is present AND the force-abort "did not drain within" warning did NOT fire — proving genuine cooperative exit, not a fallback to the backstop. <4s exit retained.

Stale-premise fixed: the AC named state.shutdown.notified() (Notify), but task-0190 migrated to a CancellationToken -> state.shutdown.cancelled().

SIGTERM stays bounded (verified ~0.3s across 10 subprocess+SIGTERM runs, wide margin under 5s). 902 workspace tests, dual gate GO/GO, zero flake. Follow-up task-0276 (test the permit-branch under a saturated semaphore). Commit 9a963c4.
<!-- SECTION:FINAL_SUMMARY:END -->
