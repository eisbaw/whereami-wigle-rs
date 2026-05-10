---
id: TASK-0052
title: >-
  Replace mark_not_found_* booleans with NotFoundPolicy enum + collapse
  drain_cleanup_after_chain
status: To Do
assignee: []
created_date: '2026-05-10 10:53'
labels:
  - refactor
  - resolver
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
ChainPolicy has two booleans (mark_not_found_per_provider, mark_not_found_at_chain_end) that are mutually exclusive in every call site but the type permits both true / both false (illegal states). Replace with NotFoundPolicy { PerProvider, AtChainEnd, Never }. Bonus: add delete_pending_on_not_found policy bit so drain_cleanup_after_chain (which currently re-queries is_not_found per BSSID after the chain just decided) collapses entirely. Found in v0.4.0 review (mped-architect, swe-gardener, keeper).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 ChainPolicy has a single NotFoundPolicy enum field, not two booleans
- [ ] #2 delete_pending_on_not_found policy bit replaces drain_cleanup_after_chain's manual cleanup
- [ ] #3 All existing chain tests still pass; new test pins the new policy combinations
<!-- AC:END -->
