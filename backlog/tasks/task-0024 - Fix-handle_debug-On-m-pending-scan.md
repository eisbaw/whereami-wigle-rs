---
id: TASK-0024
title: Fix handle_debug O(n*m) pending scan
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-05-09 21:07'
updated_date: '2026-05-09 22:33'
labels: []
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
handle_debug calls db.get_pending(1000) inside a per-BSSID loop, scanning up to 1000 entries per AP. With 30 APs and 500 pending entries that is 15000 comparisons. Also holds debouncer lock during all DB queries. Fix: query pending once into a HashSet, drop debouncer lock before DB queries.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add Database::is_pending(bssid) -> Result<bool> using SELECT 1 ... LIMIT 1.
2. Add unit test for is_pending in db.rs.
3. Replace get_pending(1000).iter().any() in handle_debug (server.rs).
4. Replace get_pending(1000) full-scan-into-set in resolve_chain mark_not_found_at_chain_end (resolver.rs) with per-bssid is_pending lookup.
5. Build / test / clippy / fmt clean in nix develop.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Added Database::is_pending(bssid) using indexed PRIMARY KEY probe. Replaced the get_pending(1000).iter().any(...) pattern in handle_debug. Also fixed the same shape in resolver::resolve_chain end-of-chain not_found marking; the prior 1000-row HashSet was not just slow, it could silently misclassify pending BSSIDs that fell outside the first 1000 rows.

All cargo build / test / clippy / fmt clean in nix develop.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Replaced the O(n*m) "scan up to 1000 pending rows per BSSID" pattern in handle_debug with an indexed Database::is_pending probe. Also fixed the same anti-pattern in resolver::resolve_chain end-of-chain not_found marking, where the truncation at 1000 was a correctness bug too: a pending BSSID outside the first 1000 rows could have been wrongly marked not_found.

Changes:
- whereamid/src/db.rs: new is_pending(bssid) -> Result<bool> using SELECT 1 ... LIMIT 1 on the PRIMARY KEY, plus unit test.
- whereamid/src/server.rs handle_debug: per-BSSID is_pending instead of get_pending(1000) full-scan.
- whereamid/src/resolver.rs mark_not_found_at_chain_end: per-BSSID is_pending instead of pre-loaded HashSet of 1000 rows.

Tests: cargo test all green (lib + integration + property).
Lint: cargo clippy --all-targets -D warnings clean.
Format: cargo fmt --check clean.
<!-- SECTION:FINAL_SUMMARY:END -->
