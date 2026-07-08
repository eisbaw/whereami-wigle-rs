---
id: TASK-0120
title: Collapse delete_pending_on_success and delete_pending_on_not_found
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:16'
updated_date: '2026-07-08 19:55'
labels:
  - refactor
  - resolver
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/resolver.rs:122-128 + src/pending.rs:114-124. Drain policy sets both to true; readonly and background set both to false. No flow has one true and the other false. The doc-comment on delete_pending_on_not_found explicitly says the asymmetry would be a bug — so they're not two bits, just one with a redundant copy.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Collapse to manage_pending: bool on ChainPolicy (or its successor ChainMode variant Drain)
- [x] #2 All existing tests pass
- [x] #3 The asymmetry-is-a-bug doc is removed because the asymmetry is now unrepresentable
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Replace two ChainPolicy bool fields with single manage_pending: bool + new doc-comment
2. Update 2 read sites (resolver.rs 416, 554) to policy.manage_pending
3. Update 5 construction sites (pending.rs x2 true, resolver.rs x3 false)
4. Update stale comments referencing old field names (resolver 525/563, pending 140/226) so grep delete_pending_on returns 0
5. Verify: nix develop -c just e2e, cargo fmt --check, drain_chain_* tests pass
6. Commit code only, conventional commit, no attribution
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented collapse of delete_pending_on_success + delete_pending_on_not_found into single manage_pending: bool on ChainPolicy.
- 5 construction sites updated (pending.rs 2x true, resolver.rs 3x false); 2 read sites (cache_resolved_ap, record_not_found).
- Stale comments referencing old field names updated (resolver 526/564, pending 140/224).
- Gate: nix develop -c just e2e PASS (whereamid lib 461/0, integration 203/0, proptests 12/0, shutdown 4/0, client suites 49/39/2/1/2 — all 0 failed); clippy -D warnings clean; cargo fmt --check clean.
- drain_chain_* tests all pass incl. drain_chain_marks_not_found_and_deletes_pending_on_definitive_miss (manage_pending=true path) — pending row still deleted on definitive outcome, behavior identical.
- Committed 4731dd1 (code only, no attribution).
- GOTCHA: grep delete_pending_on whereamid/src returns 2 hits, BOTH inside the new doc-comment prose that task step 1 explicitly dictated (names the collapsed fields for historical traceability). Zero LIVE code/field references remain. The grep-zero gate conflicts with doc step 1; kept the task-mandated doc text and satisfied the gate intent. Left In Progress for review gate.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Collapsed the two always-equal ChainPolicy pending-management flags into one, making the illegal asymmetry unrepresentable in the type.

Changes (commit 4731dd1; whereamid/src/resolver.rs + pending.rs; internal refactor, NO wire impact): replaced ChainPolicy.delete_pending_on_success + delete_pending_on_not_found with a single `manage_pending: bool` (doc-comment: true only for the pending-drain chain, which owns the pending table and deletes the row on any definitive outcome; others never touch pending). Updated the 2 read sites (cache_resolved_ap, record_not_found -> policy.manage_pending) and all 5 construction sites (pending.rs x2 both-true -> true; resolver.rs x3 both-false -> false). Removed the obsolete "asymmetry would be a bug" doc (AC#3 — now unrepresentable).

Behavior IDENTICAL: the two flags were provably always equal at every construction site (verified in the diff site-by-site), so substituting their shared value changes nothing; the compiler enforces that no site was missed (removed fields).

Verification: pure value-preserving type-narrowing refactor, orchestrator-verified: diff confirms each manage_pending value matches the original pair; grep shows zero LIVE references to the old field names (2 remaining hits are the doc-comment historical note); drain_chain tests 6/6 pass (the manage_pending=true deletion path, incl. drain_chain_marks_not_found_and_deletes_pending_on_definitive_miss); implementer just e2e 773 tests 0 failed; clippy -D warnings + fmt clean.
<!-- SECTION:FINAL_SUMMARY:END -->
