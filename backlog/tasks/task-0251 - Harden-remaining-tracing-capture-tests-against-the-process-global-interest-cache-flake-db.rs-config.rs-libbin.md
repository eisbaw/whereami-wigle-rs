---
id: TASK-0251
title: >-
  Harden remaining tracing-capture tests against the process-global
  interest-cache flake (db.rs/config.rs; lib+bin)
status: Done
assignee:
  - '@myself'
created_date: '2026-07-08 08:34'
updated_date: '2026-07-08 18:36'
labels:
  - robustness
  - test-flake
  - tech-debt
dependencies:
  - TASK-0248
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Follow-up from task-0248 (mped-architect review). task-0248 fixed the reported gate flake by installing a permissive global default subscriber (register_callsite -> Interest::sometimes()) from the three server::dispatch_tests capture tests, which makes the process-global tracing interest cache never cache a callsite as `never`. That decisively de-flaked the reported test (0/300 target failures). BUT the root cause is process-global and other capture tests share the SAME latent failure mode and are NOT covered in all run shapes:

- whereamid/src/db.rs assert_one_last_fix_drop_warning (~:3532) uses exact-count assert_eq!(all_warnings.len(), 1). db.rs is a LIB+BIN dual-target module: in the whereamid LIB test binary the server dispatch_tests never run, so the Once never installs and this test gets ZERO protection there. In the bin binary it is protected only if a server capture test runs first (order-dependent).
- config.rs (bin-only) has ABSENCE capture assertions (~:880,:901) and positive .any() captures, likewise only covered if a server test installed the global earlier in the same process.
- trilaterate.rs (lib+bin) also has a scoped-subscriber capture test.

Not currently observed flaking (0 across 360+ full-suite runs in the 0248 verification), so this is latent, not a live red. The architectural smell: a process-global mutation installed as a side effect of one test, making capture outcomes order-dependent (full `just e2e`: covered; filtered `cargo test config::` or the lib binary: not covered).

Fix direction: hoist install_permissive_global_test_subscriber() into a SHARED test-support entry point reachable from both build targets (note lib.rs vs main.rs module split) and invoke it at the top of EVERY tracing-capture test (server, db, config, trilaterate). Then the guarantee holds regardless of run shape / test order / binary. Alternatively, if a shared home is deemed not worth it, explicitly document the lib-binary db.rs:3532 residual risk at the test. Verify with an N-times (>=100) loop of BOTH the lib and bin test binaries under load.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 install_permissive_global_test_subscriber() lives in a shared test-support module reachable from both the lib and bin test targets
- [x] #2 Every tracing-capture test (server, db, trilaterate, config) invokes the installer before its scoped subscriber, so the guarantee holds regardless of run shape / test order / lib-vs-bin binary
- [x] #3 Both test binaries build+pass; lib-binary stability loop shows 0 failures
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Hoisted the task-0248 tracing-interest-cache de-flake helper into a shared, lib+bin-reachable test-support module and wired it into every capture test, closing the latent order-dependence the task-0105/0252 reviews flagged.

Changes (commit 1097a89):
- New whereamid/src/test_support.rs (#![cfg(test)], pub(crate) install_permissive_global_test_subscriber()), declared #[cfg(test)] mod test_support; under BOTH crate roots (lib.rs + main.rs). whereamid is a lib+bin crate; db.rs/trilaterate.rs compile into both test binaries, so crate::test_support::... must (and does) resolve in both — confirmed by clippy -D warnings across all targets.
- Removed the bin-only private copy from server::dispatch_tests; its 3 call sites now use crate::test_support.
- Wired the installer into every capture site: server (3), db.rs capture_events helper + the two set_last_fix/persist_fix_and_history tests, trilaterate.rs, config.rs load_with_captured_events helper. A completeness sweep (reviewer-confirmed) found NO uncovered with_default/with_subscriber/CapturedEvents/Captured site.

WHY IT IS ORDER-INDEPENDENT (reviewer-verified vs tracing-core 0.1.36 source): set_global_default -> Dispatch::new -> callsite::register_dispatch -> CALLSITES.rebuild_interest, a full re-fold of every already-registered callsite. So even a callsite cached `never` before any capture test ran is REPAIRED by the first install, and the leaked global keeps sometimes() in the fold forever after. The only requirement is install-before-with_default WITHIN each capture test, which the wiring now guarantees everywhere.

Verification: full gate green (lib 192, bin 450, workspace 741; clippy -D warnings clean confirming dual-root resolution; fmt clean); moved-helper regression (locate_skips_non_finite_cached_ap_and_warns_with_bssid) passes; LIB-binary stability loop 100/100 clean (the previously-unprotected db/trilaterate capture tests). Review qa GO / mped GO (no blockers). Since the flake was latent/0-observed, the loop confirms stability; the closure argument is the source-verified rebuild mechanism, not the loop.

Follow-ups: task-0104 (iw malformed-signal warn) is now UNBLOCKED. Filed task-0255 (consolidate the 3-4 near-identical CapturedEvents/Captured sink impls into test_support — DRY, out of this scope). Preserved (documented) honest-failure trapdoor: if future code installs a global default first, the install silently no-ops; precondition (only main() inits, not under harness) currently holds.
<!-- SECTION:FINAL_SUMMARY:END -->
