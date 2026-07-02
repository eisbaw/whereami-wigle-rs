---
id: TASK-0194
title: resolve response reports not_found for merely skipped/deferred BSSIDs
status: To Do
assignee: []
created_date: '2026-07-02 04:51'
labels:
  - correctness
  - protocol
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Provenance::NotFound is documented as definitively absent, but handle_resolve emits it for every BSSID missing from result.positioned. That bucket includes BSSIDs deferred because another task holds the in-flight claim, and BSSIDs whose only provider (WiGLE via resolve_readonly) returned Skipped because creds are missing or quota is exhausted - the daemon contacted no provider yet tells the client the BSSID is definitively unknown. Wrong on the wire under common conditions; clients that cache not_found blacklist resolvable BSSIDs. Ref whereamid/src/server.rs:698-723,237-244, resolver.rs:235-242,297-304.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 a resolve for an uncached BSSID while WiGLE is unconfigured or quota-exhausted does not return source:not_found for it
- [ ] #2 a resolve racing a background resolution of the same BSSID returns an in-progress/unresolved indication rather than not_found
- [ ] #3 not_found on the wire is emitted only when a provider or the not_found table authoritatively reported absence; docs/protocol.md matches
<!-- AC:END -->
