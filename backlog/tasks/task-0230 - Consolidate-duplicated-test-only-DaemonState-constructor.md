---
id: TASK-0230
title: Consolidate duplicated test-only DaemonState constructor
status: Done
assignee:
  - '@myself'
created_date: '2026-07-02 22:12'
updated_date: '2026-07-07 23:22'
labels:
  - robustness
  - tech-debt
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Five hand-written DaemonState struct literals exist for tests (provider.rs, pending.rs, resolver.rs, server.rs dispatch_tests) plus the production one in main.rs. Each new DaemonState field (recently shutdown/tasks/orphan_tasks_aborted from task-0190/0102) must be added in all five places, an error-prone duplication. Extract a single #[cfg(test)] pub(crate) helper (e.g. crate::server::test_daemon_state()) and route the test constructors through it.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A single shared test-only constructor builds DaemonState for tests
- [x] #2 provider/pending/resolver/server test_state helpers delegate to it
- [x] #3 Adding a DaemonState field requires editing exactly one test site
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. In server.rs make the single 28-field DaemonState literal (test_state_with_db) the sole source; add a wigle param (replacing the hardcoded WigleClient::new("","")).
2. Add pub(crate) test_state_with_wigle_client(wigle) that opens a test DB and delegates; test_state()/test_state_with_nominatim() pass a default wigle. Args debounce defaults are 10/5 == the other sites hardcoded Debouncer::new(10,5), so args-derived debouncer is behavior-identical.
3. provider.rs: 3 local helpers delegate to crate::server (removes literal). pending.rs/resolver.rs test_state() -> crate::server::test_state() (removes 2 literals). Remove now-unused imports (clap::Parser, Args) to keep clippy -D clean.
4. Gate: nix develop -c just e2e + fmt; parallel qa + mped review.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Consolidated the four duplicated test-only DaemonState literals (provider/pending/resolver + server) into a single shared constructor in server.rs. `test_state_with_db` is now the sole 28-field literal; it gained a `wigle` param. New `pub(crate) test_state_with_wigle_client(wigle)` opens its own test DB and delegates; `test_state`/`test_state_with_nominatim` pass a default WiGLE client. provider/pending/resolver test helpers now delegate to `crate::server::` and their now-unused imports (Args, AddressCache, clap::Parser) were removed.

Behavior-preserving: Args defaults are debounce_window=10/threshold=5, byte-identical to the old hardcoded Debouncer::new(10,5). Only wigle ever varied per site (provider), preserved via the new param. grep confirms exactly one hand-written test literal remains (server.rs test_state_with_db) plus production main.rs, so AC#3 holds.

Gate: full e2e green (734 tests, clippy -D clean, fmt clean). Review: qa-test-runner GO on content + mped-architect GO.

GOTCHA surfaced by QA (NOT caused by this change): the e2e gate has a pre-existing ~1-in-8 flake in `locate_skips_non_finite_cached_ap_and_warns_with_bssid` (task-0239 tracing-capture race under the multi-thread runtime). Filed separately; not attributed to task-0230.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Consolidated five duplicated test-only DaemonState constructions into one shared #[cfg(test)] constructor so a new DaemonState field is added in exactly one place.

## Changes
- server.rs: `test_state_with_db` is the single surviving 28-field literal; added a `wigle` param. New `pub(crate) test_state_with_wigle_client(wigle)`; `test_state`/`test_state_with_nominatim` pass a default WiGLE client.
- provider.rs/pending.rs/resolver.rs: local `test_state*` helpers now delegate to `crate::server::`; removed the hand-written literals and now-unused imports (Args, AddressCache, clap::Parser). Net -43 lines.

## Correctness
Behavior-identical: Args defaults (debounce_window=10, threshold=5) equal the old hardcoded Debouncer::new(10,5); every other field matched field-by-field across the old literals; provider's per-test WiGLE customization is preserved via the new param. grep proves exactly one test literal remains (+ production main.rs), so AC#3 genuinely holds.

## Gate
Full e2e green (734 tests, clippy -D warnings clean, fmt clean). Review: qa GO (content) + mped GO.

## Note
QA surfaced a pre-existing, unrelated ~1-in-8 e2e flake in task-0239's tracing-capture test; filed as a separate follow-up, not fixed here.
<!-- SECTION:FINAL_SUMMARY:END -->
