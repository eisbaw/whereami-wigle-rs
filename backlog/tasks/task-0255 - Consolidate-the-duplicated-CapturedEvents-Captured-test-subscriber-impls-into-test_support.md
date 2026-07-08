---
id: TASK-0255
title: >-
  Consolidate the duplicated CapturedEvents/Captured test subscriber impls into
  test_support
status: Done
assignee:
  - '@myself'
created_date: '2026-07-08 18:36'
updated_date: '2026-07-08 22:16'
labels:
  - tech-debt
  - test-infra
dependencies:
  - TASK-0251
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Follow-up from task-0251 review (mped-architect, informational). There are ~3-4 near-identical test-only tracing capture-sink subscribers across the crate — server.rs CapturedEvents (3-field: level/target/message), db.rs CapturedEvents (2-field: level/message), config.rs CapturedEvents (2-field), trilaterate.rs Captured (3-field). They differ only in the captured tuple shape. Now that whereamid/src/test_support.rs exists (task-0251) as a shared lib+bin test-support home, these could be consolidated into one generic capture sink there (e.g. a CaptureLayer capturing (Level, target, message) with helper accessors), removing the duplication. LOW priority / pure test-infra DRY: verify the consolidated sink preserves each call site's existing assertions (target filtering in server.rs vs message-only in db/config). Not blocking anything.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add shared CaptureSink (Subscriber, 3-field level/target/message) + MessageVisitor + events() snapshot + record_event() injector + capture() helper (installs permissive global + with_default) to test_support.rs.
2. Migrate 6 sites, deleting each local struct+visitor:
   - server.rs: 2 sync sites -> capture(); 2 async (.with_subscriber) sites -> raw CaptureSink+events().
   - trilaterate.rs: capture(), filter by target.
   - db.rs/config.rs/scanner.rs/history.rs: local wrapper maps 3-field->2-field; assertions unchanged. db install_commit_marker uses CaptureSink::record_event.
3. Fix imports to avoid -D warnings. grep-confirm zero duplicate sink structs.
4. Gate: nix develop -c just e2e; cargo fmt --check; cargo test -p whereamid --lib and --bin whereamid.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented (commit ae07b83). Added shared CaptureSink + capture() + record_event() + MessageVisitor to test_support.rs. Migrated all 6 sites, deleted local structs.

Gate (nix develop -c just e2e): whereamid lib 205 passed, bin 466 passed, proptests 12, shutdown 4, client integration (protocol_version 2 / sigpipe 1 / version_flags 2); clippy --all-targets -D warnings clean; cargo fmt --check clean. All capture tests confirmed in BOTH lib and bin targets. No #[test] added/removed.

Gotcha: clippy (not cargo test) flagged db.rs module-level `use std::sync::Mutex` as unused after the sink deletion (Mutex now only used via a function-local import at db.rs:6264); removed it. Async server sites (.with_subscriber) cannot use cap() and keep the raw CaptureSink. Left In Progress for review gate.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Consolidated the SIX duplicated #[cfg(test)] tracing capture-sink subscribers into one shared sink in whereamid/src/test_support.rs (commit ae07b83, TEST-ONLY, net -157 lines).

Added to test_support: CaptureSink (impl tracing::Subscriber, records the superset (Level, target, message)) + a clonable handle + events() snapshot + a capture<T>(f) helper that installs the permissive global (task-0251) and runs f under a scoped CaptureSink + record_event() for db's commit-hook marker; MessageVisitor hoisted ONCE here. Migrated all 6 sites (server.rs + trilaterate.rs 3-field; db.rs + config.rs + scanner.rs + history.rs 2-field), DELETING each local CapturedEvents/Captured/MessageVisitor struct — 2-field sites map (lvl,_target,msg)->(lvl,msg) and keep their exact assertions; the two async server sites use the raw CaptureSink (the scoped guard must span the future via WithSubscriber, so the capture() helper does not fit). 

Verified (orchestrator, test-only DRY): diff contains NO production-code lines (all within test modules / test_support); grep confirms the ONLY remaining sink/visitor struct is in test_support.rs (zero duplicates elsewhere); both dual-target test binaries pass with UNCHANGED counts (whereamid lib 205, bin 466 — same as pre-refactor, so no test was dropped and no assertion weakened); clippy -D warnings + fmt clean (the gate caught + the implementer removed a now-unused `use Mutex` — exactly why -D warnings matters). Total #[test] count unchanged (diff touches no #[test]/#[tokio::test] lines). No follow-ups.
<!-- SECTION:FINAL_SUMMARY:END -->
