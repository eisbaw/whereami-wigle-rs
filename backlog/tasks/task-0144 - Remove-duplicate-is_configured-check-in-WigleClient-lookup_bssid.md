---
id: TASK-0144
title: 'Remove duplicate is_configured check in WigleClient::lookup_bssid'
status: To Do
assignee: []
created_date: '2026-05-20 21:26'
labels:
  - hygiene
  - refactor
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/wigle.rs:88 + src/provider.rs:120. Both gates check is_configured. Provider returns Skipped; WigleClient returns WigleError::Network. Two different outcomes for the same precondition violation — second can only fire if someone bypasses the provider abstraction, which today no one does. Dead defense.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Remove the is_configured check from WigleClient::lookup_bssid (or downgrade to debug_assert!)
- [ ] #2 Doc-comment on WigleClient::lookup_bssid: 'caller must have verified is_configured(); use Provider abstraction'
- [ ] #3 All existing tests pass
<!-- AC:END -->
