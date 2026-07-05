---
id: TASK-0192
title: >-
  Move blocking rusqlite off the async runtime; set synchronous=NORMAL +
  busy_timeout
status: Done
assignee:
  - '@myself'
created_date: '2026-07-02 04:51'
updated_date: '2026-07-05 01:29'
labels:
  - correctness
  - performance
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Database sits behind std::sync::Mutex and every call runs directly on tokio workers (no spawn_blocking anywhere). One locate does up to 10 touch_ap UPDATEs + set_last_fix + insert_fix ~= 12 autocommit write txns. With SQLite default synchronous=FULL each commit fsyncs the WAL (5-30ms on SBC/SD-card hardware) -> 100-350ms per locate holding the global lock, blocking every other handler's OS worker; a concurrent daily prune_fixes DELETE or inline WAL checkpoint can freeze the whole runtime incl timers and the accept loop. No busy_timeout is set (external sqlite writer -> instant SQLITE_BUSY, data silently dropped). try_reserve_api_call doc claims IMMEDIATE but uses a DEFERRED transaction. Ref whereamid/src/server.rs:74-78, resolver.rs:141-147, db.rs:104-123,656-667,888-894.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 no rusqlite call executes on a tokio worker thread (spawn_blocking or a dedicated DB thread)
- [x] #2 a locate touching 10 cached APs performs at most 1-2 write transactions
- [x] #3 PRAGMA synchronous=NORMAL and a busy_timeout are set; concurrent stats/version stay responsive under a slowed DB or large prune
- [x] #4 try_reserve_api_call uses (or documents) the correct transaction mode
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add cloneable async DbHandle access and lifecycle-owning DbRuntime backed by dedicated writer and WAL-reader OS threads; open/migrate writer after signals, then open the no-create reader.
2. Preserve chmod-before-WAL ordering; configure synchronous=NORMAL, a named 2s busy timeout, foreign keys, and IMMEDIATE quota transactions.
3. Batch cached lookup+touch and last_fix+history so a ten-AP production locate commits exactly twice; use index-driven 1000-row prune chunks.
4. Route production persistence through cancellation-aware actor commands. Use interruptible reader work, durable last-fix writes, and a pre-await QuotaLease whose same-FIFO conditional refund is exactly once.
5. Close lifecycle races: track handlers under the shutdown lock, reap quota-owning tasks before actor Stop, independently own explicit shutdown cleanup, and provide idempotent DbRuntime/DbActor fallback Drop cleanup for error and unwind paths.
6. Prove behavior with production-path commit hooks, actor-thread/pragmas/contention tests, quota cancellation gates, task/shutdown races, indexed prune plans/chunk counts, startup/drop cleanup, full E2E, and parallel QA/architecture review.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from task-0197: db.rs Database::open() now calls tighten_db_file_permissions(path) (chmod DB file to 0600) IMMEDIATELY after Connection::open and BEFORE the journal_mode=WAL pragma. This ordering is load-bearing: SQLite copies the main DB files perms onto the -wal/-shm sidecars it creates on WAL activation, so the chmod must precede WAL. When you add synchronous=NORMAL + busy_timeout pragmas (and/or move rusqlite onto spawn_blocking), keep the tighten call before the journal_mode pragma. There is an in-code NOTE(task-0192) marking this constraint at the call site.

Forward-carried from task-0237: daemon startup now depends on an ORDERING INVARIANT — the SIGTERM/SIGINT handlers are installed (main.rs, right after args.validate()) BEFORE Database::open + migrations and BEFORE the control-socket bind. Two consequences for this task: (1) tests/shutdown.rs uses "socket file exists" as a readiness proxy that is only sound because handler-install precedes the bind; if you move DB open/migrations onto a spawn_blocking thread or otherwise reorder startup, keep handlers installed before the socket bind. (2) A SIGTERM arriving during startup is currently DEFERRED (latched) until startup finishes, then drained — acceptable for tiny migrations. If migrations become long-running/blocking under this task, revisit whether startup should race against the signal instead of deferring, and re-run the shutdown.rs sigterm test 20x under load.

2026-07-05 reconnaissance: chose separate writer and WAL-reader actors. A single actor or spawn_blocking+global mutex does not satisfy literal stats responsiveness during prune. Database stays actor-owned; no rusqlite object escapes db.rs. DbRuntime owns thread shutdown and remains alive until all daemon work and shutdown metadata persistence are drained. TASK-0201 concurrency caps remain out of scope, but any handler holding a quota guard must be drained before DB shutdown.

2026-07-05 completion correction: the final implementation uses an owned QuotaLease, not the earlier planned QuotaSlot. Per-connection handlers are now tracked and drained for actor-lifecycle safety; admission/concurrency caps and the existing O(n^2) history CPU path remain explicitly deferred to TASK-0201/TASK-0087. Exceptional DbRuntime/DbActor fallback Drop may synchronously wait for active actor work, but SQLite creation/use/destruction remains confined to actor threads; normal explicit shutdown uses an independently owned blocking cleanup task.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Implemented a dual-actor SQLite runtime with writer/read-WAL separation, NORMAL synchronous mode, explicit 2s busy timeout, IMMEDIATE quota transactions, cancellation-aware commands, cancellation-safe exactly-once QuotaLease refunds, and fully joined startup/shutdown/error lifecycles. Batched cached touches plus fix persistence reduce a ten-AP locate to exactly two commits; prune uses bounded 1000-row timestamp-indexed chunks. Production raw Database access is sealed. Deterministic lifecycle, cancellation, contention, query-plan, and full locate regressions were added. Final independent QA and architecture gates both returned GO; the reviewed suite contains 383 passing tests plus strict Clippy.
<!-- SECTION:FINAL_SUMMARY:END -->
