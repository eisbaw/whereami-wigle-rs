---
id: TASK-0274
title: Cap the scanner already-warned malformed-BSSID set (unbounded in principle)
status: Done
assignee:
  - '@myself'
created_date: '2026-07-09 05:48'
updated_date: '2026-07-09 08:23'
labels:
  - scanner
  - observability
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
task-0256 review LOW: scanner.rs malformed_signal_warn_is_first uses a never-cleared process-global OnceLock<Mutex<HashSet<String>>> keyed iw:/nmcli:<bssid>. Bounded in practice (one small entry per distinct malformed-signal BSSID the LOCAL driver ever emits; the malformed field is local-driver-sourced, not attacker-drivable over the air), but unbounded in principle over a very long daemon lifetime in an RF environment cycling many distinct malformed BSSIDs. Optional hardening: cap the set (once size > N, always DEBUG) or LRU-evict. LOW — the adversarial path is constrained; current behavior is acceptable + documented.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 The already-warned set cannot grow without bound: either a size cap (beyond N, malformed-signal repeats always DEBUG) or an eviction policy, with a test
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Done in-thread (small self-contained hardening, my own 0256 follow-up). Capped the already-warned set at MAX_WARNED_KEYS=1024: once that many distinct keys have each warned once, a NEW key returns false (DEBUG) and is NOT inserted -> the OnceLock<Mutex<HashSet>> stops growing. Extracted the pure warn_is_first_in(seen, key, max) so the cap is testable deterministically (small max, fresh local set, no process-global contamination). Test warn_is_first_in_caps_the_set_and_dedups BITES: without the len>=max check "d" would grow the set to 4 -> assert_eq!(seen.len(),3) fails. Existing 0256 malformed-signal tests still pass (their keys are under the cap). Gate: cap test + parse_iw_output_malformed_signal_warns_once_then_debug pass; just e2e 16/16 binaries ok; clippy -D warnings clean; fmt clean. No behavior change for real environments (a handful of broken APs, far under 1024).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Bounded the scanner\x27s never-cleared already-warned malformed-signal set (task-0256 review LOW). Capped at MAX_WARNED_KEYS=1024 via a pure, testable warn_is_first_in(seen, key, max) helper: beyond the cap a new malformed-signal key is demoted straight to DEBUG and not inserted, so the process-global set cannot grow without bound over a very long daemon lifetime. A cap test asserts the set stops growing past the cap (bites the missing len>=max check); existing malformed-signal tests unaffected (keys under the cap). No behavior change for real environments. Gate green (fmt/clippy/e2e). Commit: (fix scanner).
<!-- SECTION:FINAL_SUMMARY:END -->
