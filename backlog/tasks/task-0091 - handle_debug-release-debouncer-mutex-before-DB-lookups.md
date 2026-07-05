---
id: TASK-0091
title: 'handle_debug: release debouncer mutex before DB lookups'
status: Done
assignee:
  - task0091-implementer
created_date: '2026-05-13 21:55'
updated_date: '2026-07-05 11:23'
labels:
  - robustness
  - server
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
server.rs::handle_debug acquires the (tokio) debouncer mutex at the top, then iterates over all_bssids doing three sequential SQL queries per BSSID (get_ap, is_not_found, is_pending), each acquiring the std DB mutex. With a busy debouncer (20+ visible BSSIDs) that is 60+ index probes, all while blocking handle_locate and handle_scan from acquiring the debouncer. 'watch -n1 whereami debug' on a multi-AP environment will interfere with locate latency. Fix: snapshot the data needed out of the debouncer into a local Vec, release the guard, then run DB lookups outside the lock. Or batch the DB queries into one statement (SELECT bssid,... FROM aps WHERE bssid IN (?,?,?)) so the std DB lock is held once. The snapshot-then-release pattern is simpler and matches what handle_stats already does.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 handle_debug acquires the debouncer mutex, snapshots the BSSID list + counters into a local Vec, then drops the guard before any DB calls
- [x] #2 DB lookups happen with the debouncer NOT held
- [ ] #3 Stretch (optional): batch get_ap / is_not_found / is_pending into single IN-clause queries so the DB mutex is taken once per query type, not once per BSSID
- [x] #4 Test: concurrent handle_debug + handle_locate (e.g. spawn both, race them, assert neither blocks the other beyond a small bound)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Preserve the existing snapshot/drop/batched-debug_statuses production implementation introduced by commit e050718.
2. Expose only the existing cfg(test) reader-blocking seam as pub(crate) if needed by server tests.
3. Add a deterministic multi-thread test: occupy the DB reader, queue handle_debug behind a held debouncer guard, release it, prove handle_debug remains DB-stalled while handle_scan (or equivalent debouncer-only handler) completes promptly, then release and join all work.
4. Run focused/full tests, mandatory parallel reviews, final E2E, and commit the regression coverage plus tracker closure.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Current-code audit: commit e050718 (TASK-0192 DB actor refactor) already snapshots debug inputs, explicitly drops the debouncer, and performs one batched DbHandle::debug_statuses read. TASK-0091 remains open because no direct lock-scope concurrency regression pins that behavior.

Completion (2026-07-05): production commit e050718 already snapshots all debouncer-derived debug inputs, explicitly drops the Tokio mutex guard, then awaits one batched DbHandle::debug_statuses reader-actor request. Added a deterministic concurrency regression that occupies the real DB reader, FIFO-queues handle_debug on the debouncer, and proves DB-independent handle_scan completes while debug remains stalled. A disposable mutation restoring the old lock-across-DB-await scope makes the exact test fail at the scan bound. Kept optional AC #3 unchecked: debug_statuses batches the reader-actor round trip but still performs indexed per-BSSID probes rather than literal SQL IN queries. QA and architecture reviews: GO at frozen diff f126c8b116f526d85b511a9e0737f24fdaa01b3ca140b89ae88581f7a3d5b82c; uncontaminated stress runs passed 600/600 plus the full suite.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Pinned the existing handle_debug lock-scope fix with deterministic concurrency coverage. Debug now has regression protection proving it releases the debouncer before a stalled DB read, allowing scan requests to proceed; the test-only DB reader hook remains cfg(test). Mandatory QA and architecture gates passed. Optional SQL IN batching remains intentionally out of scope and unchecked.
<!-- SECTION:FINAL_SUMMARY:END -->
