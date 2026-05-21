---
id: TASK-0104
title: 'parse_iw_output: warn on malformed signal line (parity with nmcli)'
status: To Do
assignee: []
created_date: '2026-05-20 21:11'
labels:
  - robustness
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/scanner.rs:252-258. If 'signal: <garbage> dBm' appears mid-block, the line is silently skipped, current_signal stays None, and the BSSID block is dropped at the next BSS boundary. task-0043 fixed this for nmcli; iw parser has the same shape but no warn. Driver corner-cases (e.g. signal printed as 'unknown') would silently drop APs without observability.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 On signal-line parse failure, emit tracing::warn (rate-limited to once per BSS block) with the offending line excerpt
- [ ] #2 BSS is still dropped (correct behavior), but operator can see why
- [ ] #3 Unit test: parse_iw with one malformed signal line in an otherwise-valid block; assert log captured + BSS dropped
<!-- AC:END -->
