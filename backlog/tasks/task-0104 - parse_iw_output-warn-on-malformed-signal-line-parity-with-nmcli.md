---
id: TASK-0104
title: 'parse_iw_output: warn on malformed signal line (parity with nmcli)'
status: To Do
assignee: []
created_date: '2026-05-20 21:11'
updated_date: '2026-07-08 18:21'
labels:
  - robustness
dependencies:
  - TASK-0251
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Triaged 2026-07-08 (not implemented — deferred behind task-0251):
1. PREMISE CORRECTION: the task says task-0043 added a malformed-signal warn to the nmcli parser and asks for "parity". But the CURRENT nmcli parser (scanner.rs:~719) silently `continue`s on an unparseable signal with NO warn (comment "Skip rows where signal is missing or unparseable"). So there is no nmcli warn to mirror. The iw-parser observability gap (scanner.rs:~899-903: malformed `signal:` line -> current_signal stays None -> BSS dropped at next boundary, silently) is STILL a real robustness improvement worth doing — just re-frame away from "parity with nmcli" (and consider adding the same warn to the nmcli continue-site for real parity).
2. AC#3 ("assert log captured") is entangled with the tracing-capture flake: parse_iw_output is a pure fn, so verifying its warn! needs a scoped subscriber, and scanner tests run in the whereamid LIB test binary alongside db.rs/trilaterate.rs capture tests -> the same process-global interest-cache clobber task-0248 fixed for the BIN binary and task-0251 tracks for the LIB side. A naive capture test here would be flake-prone. => depends on task-0251 (shared permissive-global test-subscriber reachable from lib-target modules), OR re-scope AC#3 to a deterministic assertion (e.g. assert BSS dropped only, or expose a malformed-line diagnostic).
Recommend: do 0251 first (hoist install_permissive_global_test_subscriber to a lib+bin-reachable test-support module), then this becomes a clean warn+capture task.
<!-- SECTION:NOTES:END -->
