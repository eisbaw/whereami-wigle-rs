---
id: TASK-0042
title: 'Validate Args at parse time, not runtime panic'
status: Done
assignee:
  - '@claude'
created_date: '2026-05-10 05:38'
updated_date: '2026-05-10 07:17'
labels:
  - config
  - reliability
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
debounce::Debouncer::new asserts threshold <= window and panics if violated. CLI accepts --debounce-threshold and --debounce-window as independent values, so a user can crash the daemon at startup with bad config. Move the validation into a clap value_parser or a custom Args::validate() called immediately after parse, surfacing a clean error before any side effects. Audit other modules for similar runtime panics on user input.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Invalid --debounce-threshold/--debounce-window combinations produce a clean error message at startup, not a panic
- [x] #2 Audit recorded in task notes: list of all assert!() / panic!() that depend on user-supplied Args values
- [x] #3 Test covers an invalid combination producing a non-zero exit and a useful message
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Audit assert!() / panic!() that depend on user-supplied Args
2. Add Args::validate() called immediately after Parse, surfacing a clean error before any side effects
3. Specifically: debounce_threshold <= debounce_window, scan_interval_fast > 0, scan_interval_slow > 0, scan_fast_duration sane, top_n > 0
4. Test for invalid combination produces non-zero exit message
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Audit: only one production-code assert depended on user input — debounce.rs:36 'threshold <= window'. The Debouncer::new assert is preserved as a defence-in-depth invariant (it is correct that the assert holds; the issue is when it fires).

Added Args::validate() called immediately after Args::parse() in main.rs. Validates: debounce_window > 0, debounce_threshold > 0, debounce_threshold <= debounce_window, scan_interval_fast > 0, scan_interval_slow > 0, top_n > 0, pending_interval > 0, pending_max_attempts > 0, not_found_ttl_days > 0. Each check produces a clean anyhow::bail with the offending flag name in the message.

6 unit tests in config::tests covering defaults, threshold>window rejection, threshold==window acceptance, and zero rejections for window/top_n/scan_interval_fast.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added Args::validate() called immediately after parse(), before any side effects (DB open, signals, threads). Bad combinations (e.g. --debounce-threshold 100 --debounce-window 5) produce a clean anyhow error message naming the offending flag, instead of crashing inside Debouncer::new with a runtime assert. Defence-in-depth: the Debouncer assert is preserved. 6 new unit tests.
<!-- SECTION:FINAL_SUMMARY:END -->
