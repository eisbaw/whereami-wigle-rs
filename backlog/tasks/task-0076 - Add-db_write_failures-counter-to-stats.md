---
id: TASK-0076
title: Add db_write_failures counter to stats
status: Done
assignee: []
created_date: '2026-05-10 11:01'
updated_date: '2026-05-10 14:10'
labels:
  - observability
  - server
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
23 'best-effort write failed' warn! sites silently allow DB writes to fail while logs are the only record. If last_fix or fixes ever become load-bearing (or storage fills up) operators have no quick way to ask 'is the DB writable?'. Expose db_write_failures_total via stats. Found in v0.4.0 review (keeper).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 DaemonState carries an AtomicU64 db_write_failures_total
- [x] #2 Every warn-and-continue DB write site bumps the counter
- [x] #3 stats response includes db_write_failures_total
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Added db_write_failures: AtomicU64 to DaemonState. record_db_failure() helper bumps the counter. Sites instrumented: server.rs (last_fix persist, history insert, address backfill), pending.rs (delete_expired, delete_not_found, insert_pending re-pend, get_expired_not_found), resolver.rs (upsert_ap, delete_pending on success, insert_not_found per-provider, insert_pending end-of-loop, insert_not_found end-of-chain, delete_pending after not_found mark, increment_pending_attempts on network error). 14 sites total. CLI renders 'db write fail: N' in stats.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cumulative best-effort DB-write-failure counter exposed via stats. Every warn-and-continue DB write site now bumps the counter; silent corruption is visible without scraping the journal.
<!-- SECTION:FINAL_SUMMARY:END -->
