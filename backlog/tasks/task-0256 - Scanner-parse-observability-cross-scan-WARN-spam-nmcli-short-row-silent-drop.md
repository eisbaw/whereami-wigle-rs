---
id: TASK-0256
title: >-
  Scanner parse observability: cross-scan WARN spam + nmcli short-row silent
  drop
status: Done
assignee:
  - '@myself'
created_date: '2026-07-08 18:47'
updated_date: '2026-07-09 05:48'
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

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Cross-scan WARN spam fixed: a chronically-malformed scanner signal line no longer emits a WARN on every scan for the daemon lifetime (first-occurrence-then-DEBUG, a process-lifetime already-warned set, or equivalent); a test feeds repeated malformed input and asserts the WARN fires once, not per-scan
- [x] #2 nmcli short-row (fields.len()<5) silent drop now emits at least a debug! naming the offending row, for parity with the signal-line observability; a test asserts the log
- [x] #3 Log assertions use crate::test_support::install_permissive_global_test_subscriber() (task-0251) for reliable capture; no new flaky test
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add process-lifetime already-warned set (OnceLock<Mutex<HashSet<String>>>) helper malformed_warn_is_first(key)->bool in scanner.rs.
2. iw signal warn (~916) and nmcli signal warn (~721): first occurrence WARN (keyed by source-tagged BSSID), repeats DEBUG. Preserve message substrings the existing tests assert.
3. nmcli short-row (~702 fields.len()<5): add debug! naming the row.
4. Tests via test_support::capture: (a) spam test drives parse_iw_output twice same unique BSSID -> WARN once then DEBUG; (b) short-row debug test. Unique BSSIDs to avoid process-global set contamination -> non-flaky.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Fixed both scanner observability gaps. (1) Cross-scan WARN spam: malformed_signal_warn_is_first(key) backed by OnceLock<Mutex<HashSet<String>>>, keyed iw:/nmcli:<bssid> — first malformed-signal occurrence WARNs, repeats DEBUG. Applied to both iw (scanner.rs:986) and nmcli (767) sites; drop behavior UNCHANGED. (2) nmcli short-row (fields.len()<5) now emits a debug! naming the row (scanner.rs:738), was a silent continue. Tests: parse_iw_output_malformed_signal_warns_once_then_debug (bites: feeds same input twice, asserts first=1 WARN, second=0 WARN + 1 DEBUG) + parse_nmcli_output_debug_logs_truncated_short_row; via test_support::capture. Order-independent (unique BSSIDs 56:01/56:02 vs siblings 24:21/24:31; capture is thread-local). QA: 5x isolated + 5x full-module-parallel + 2x full-suite = zero flake. Dual gate GO/GO (876).

LOW (filed task-0274): the already-warned set is unbounded-in-principle (bounded in practice; malformed field is local-driver-sourced, not attacker-drivable). LOW (noted): the new tests ride on the task-0248 tracing-capture infra which has a separate open flake — inherited, not introduced here.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Two scanner-parser observability fixes (task-0104 follow-up). (1) A chronically-malformed driver signal line no longer WARN-spams every scan for the daemon lifetime: a process-lifetime already-warned set (keyed by source-tagged BSSID) WARNs the first occurrence and DEBUGs repeats. (2) A truncated nmcli row (fields<5), previously dropped silently, now emits a debug! naming the row, for parity with the signal-line observability. Drop behavior is unchanged — only logging. Two non-flaky tests (order-independent unique BSSIDs; QA 5x+ no flake). Follow-up task-0274 filed to cap the unbounded-in-principle already-warned set. Dual gate GO/GO (876). Commit 2667077.
<!-- SECTION:FINAL_SUMMARY:END -->
