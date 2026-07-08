---
id: TASK-0259
title: Add idx_not_found_checked_at index (prune + revive are full-table scans)
status: To Do
assignee: []
created_date: '2026-07-08 20:08'
labels:
  - performance
  - db
  - tech-debt
dependencies:
  - TASK-0110
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Follow-up from task-0110 review (mped-architect, MEDIUM). There is NO index on not_found.checked_at (confirmed indexes: idx_fixes_at, idx_aps_geo, idx_aps_last_seen). So BOTH prune_not_found (DELETE ... WHERE checked_at < ?, every drain pass, task-0110) AND revive_expired_not_found (SELECT ... WHERE checked_at <= ? LIMIT 5, every drain pass) are FULL-TABLE SCANS on the exact table task-0110 says grows huge — and they run under the single-writer DbHandle.write lock, so a long scan blocks record_not_found/set_last_fix/pending writes in the writer FIFO. Inconsistent with precedent: fixes/aps got expiry/range indexes AND query-plan assertion tests (db.rs ~3684/3742 "must SEARCH idx_..."). Fix: add a schema migration v7->v8 creating idx_not_found_checked_at, and a query-plan assertion test (EXPLAIN QUERY PLAN shows SEARCH via the index for both the prune DELETE and the revive SELECT). Makes both O(log N + matches) instead of O(N)/pass. Mirror the existing idx_fixes_at migration + query-plan test pattern.
<!-- SECTION:DESCRIPTION:END -->
