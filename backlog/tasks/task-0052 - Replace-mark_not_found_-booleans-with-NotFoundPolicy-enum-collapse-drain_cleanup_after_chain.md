---
id: TASK-0052
title: >-
  Replace mark_not_found_* booleans with NotFoundPolicy enum + collapse
  drain_cleanup_after_chain
status: Done
assignee: []
created_date: '2026-05-10 10:53'
updated_date: '2026-05-10 13:40'
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
- [x] #1 ChainPolicy has a single NotFoundPolicy enum field, not two booleans
- [x] #2 delete_pending_on_not_found policy bit replaces drain_cleanup_after_chain's manual cleanup
- [x] #3 All existing chain tests still pass; new test pins the new policy combinations
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Replaced two booleans (mark_not_found_per_provider + mark_not_found_at_chain_end) with NotFoundPolicy enum (PerProvider / AtChainEnd / Never). Added delete_pending_on_not_found policy bit; resolve_chain now does the delete_pending in the same critical section when it marks not_found. drain_cleanup_after_chain function REMOVED entirely — its responsibility now lives in resolve_chain via the policy bit. drain_once policy uses delete_pending_on_not_found=true. Tests updated and renamed (drain_cleanup_* -> drain_chain_*); behavior unchanged externally.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
NotFoundPolicy enum + delete_pending_on_not_found bit collapse the prior two-boolean illegal-state space and eliminate drain_cleanup_after_chain. resolve_chain owns the full not_found+pending lifecycle in one critical section.
<!-- SECTION:FINAL_SUMMARY:END -->
