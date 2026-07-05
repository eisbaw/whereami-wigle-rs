---
id: TASK-0223
title: >-
  Make the Kani haversine harness assert finiteness instead of vacuous
  panic-freedom
status: Done
assignee: []
created_date: '2026-07-02 04:55'
updated_date: '2026-07-05 09:40'
labels:
  - testing
  - math
dependencies:
  - TASK-0185
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The existing geo.rs Kani harness `haversine_m_no_panic_on_finite_input` only calls `haversine_m` and discards the result. The function has no panicking operation, so pure-f64 panic-freedom is true by construction and the proof establishes no useful postcondition. The actual defect on this surface was NaN output (fixed by TASK-0185), whose downstream panic occurred in median(). Reference: whereamid/src/geo.rs.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 the geo.rs harness asserts finiteness/non-negativity of the output under an in-range-degrees assume, or is removed with rationale
- [x] #2 harness passes after the haversine clamp fix
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Attempt the non-vacuous bounded-coordinate Kani postcondition against the real haversine implementation.
2. Record the verifier result honestly: Kani 0.67 cannot model f64::asin and leaves the float postconditions undetermined.
3. Use the acceptance criterion alternative: remove the vacuous harness with an in-code rationale instead of introducing unsound transcendental stubs.
4. Run focused antipode/proptest coverage and exact just e2e; attempt the full Kani suite and report its actual scalability result.
5. Obtain parallel QA and architecture reviews, resolve findings, and commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
The first focused proof attempt was initially blocked by a stale rustup linker wrapper referencing a garbage-collected Nix store path. Reinstalling the exact pinned nightly repaired the environment. The real bounded proof then ran but verified 0/2 duplicate targets: f64::asin is unsupported and trig/NaN postconditions were UNDETERMINED, not counterexamples. A stubbed proof was rejected as unsound. Exact just kani was attempted and terminated with exit 130 after more than 13 minutes without a verdict from its first pre-existing Apple decoder harness; CBMC remained CPU-bound expanding std/tracing/backtrace teardown loops. Exact E2E and executable antipode/proptest coverage pass.

Mandatory qa-test-runner and mped-architect reviews returned GO on frozen diff 5a805cc421e1d7031beb76d15fd31381e2f569b5d46c9d1ebe534e663d7af028. AC1 is satisfied through the explicit removal-with-rationale path. AC2 is not applicable once the harness is removed and is not claimed as a passing proof. Focused antipode tests and the near-antipodal proptest pass; full E2E passes 495 tests with strict clippy.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Removed the vacuous haversine Kani panic-freedom harness after a real bounded postcondition attempt demonstrated Kani 0.67 cannot model f64::asin and leaves the required float properties undetermined. Documented why transcendental stubs would be unsound and pointed to exact/near-antipode unit tests plus the near-antipodal proptest. Production math is unchanged. The full Kani suite was honestly reported as non-completing in a pre-existing Apple harness; E2E, focused tests, QA, and architecture review all pass.
<!-- SECTION:FINAL_SUMMARY:END -->
