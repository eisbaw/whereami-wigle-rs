---
id: TASK-0101
title: prune_fixes(0) deletes all fixes instead of disabling retention
status: To Do
assignee: []
created_date: '2026-05-20 21:10'
labels:
  - correctness
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/db.rs:888-894 + src/config.rs:90-91. --history-retention-days default 30, validated >=0, help text says '0 disables retention'. That comment is honest about the bug — prune_fixes(0) computes cutoff=now and deletes everything older than now. So 0 means 'wipe history every prune sweep', not 'keep forever'. Surprising and contradicts help text.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 prune_fixes early-returns Ok(0) when retention_days == 0 (or main.rs history-prune skips the call)
- [ ] #2 Help text for --history-retention-days updated to match: '0 disables retention (no pruning)'
- [ ] #3 Unit test: with retention_days=0 and 10 old fixes in DB, prune_fixes is a no-op (fixes table unchanged)
<!-- AC:END -->
