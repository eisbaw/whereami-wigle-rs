---
id: TASK-0251
title: >-
  Harden remaining tracing-capture tests against the process-global
  interest-cache flake (db.rs/config.rs; lib+bin)
status: To Do
assignee: []
created_date: '2026-07-08 08:34'
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
