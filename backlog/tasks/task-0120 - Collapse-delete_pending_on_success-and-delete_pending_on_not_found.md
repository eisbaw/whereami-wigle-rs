---
id: TASK-0120
title: Collapse delete_pending_on_success and delete_pending_on_not_found
status: To Do
assignee: []
created_date: '2026-05-20 21:16'
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
- [ ] #1 Collapse to manage_pending: bool on ChainPolicy (or its successor ChainMode variant Drain)
- [ ] #2 All existing tests pass
- [ ] #3 The asymmetry-is-a-bug doc is removed because the asymmetry is now unrepresentable
<!-- AC:END -->
