---
id: TASK-0256
title: >-
  Scanner parse observability: cross-scan WARN spam + nmcli short-row silent
  drop
status: To Do
assignee: []
created_date: '2026-07-08 18:47'
labels:
  - robustness
  - observability
dependencies:
  - TASK-0104
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Follow-up from task-0104 review (mped-architect, non-blocking). Two remaining scanner-parser observability gaps:

1. CROSS-SCAN WARN SPAM (M1): task-0104 added a warn on a malformed iw/nmcli signal line. That is once-per-block WITHIN a single parse, but parse_iw_output/parse_nmcli_output run every scan cycle in the long-running daemon, so a chronically-malformed driver (e.g. always prints "signal: unknown dBm" for one AP) emits the WARN on EVERY scan for the daemon lifetime — steady WARN-level log spam. Fix direction: first-occurrence-then-demote-to-DEBUG, or a process-lifetime "already-warned malformed BSSIDs" set, or periodic summary. scanner.rs ~916 (iw) and ~721 (nmcli).

2. NMCLI SHORT-ROW SILENT DROP: scanner.rs ~702 `if fields.len() < 5 { continue; }` drops a malformed/truncated nmcli row with NO log at all — the same "AP silently vanishes" class task-0104 just fixed for the signal field, arguably more likely in the wild (truncated nmcli output). Add at least a debug! (or warn!) naming the offending row, for parity with the signal-line observability.

Both are LOW/MEDIUM operational-observability improvements, deterministic to test (feed malformed input, assert the log). Test the log via crate::test_support::install_permissive_global_test_subscriber() (task-0251) for reliable capture.
<!-- SECTION:DESCRIPTION:END -->
