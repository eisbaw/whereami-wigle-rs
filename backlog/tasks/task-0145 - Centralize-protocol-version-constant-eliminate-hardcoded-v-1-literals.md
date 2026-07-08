---
id: TASK-0145
title: 'Centralize protocol version constant (eliminate hardcoded v: 1 literals)'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:27'
updated_date: '2026-07-08 19:17'
labels:
  - hygiene
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Hardcoded 'v': 1 protocol version literal scattered across every response struct — ~8 sites in server.rs, every wire struct in client/lib.rs. When v=2 happens, sed-edit in 16+ places. A const WIRE_VERSION: u32 = 1; referenced everywhere is one-line free insurance.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Define const WIRE_VERSION: u32 = 1; in whereami-protocol crate (or shared module if 0114 hasn't landed yet)
- [x] #2 All response struct construction sites use WIRE_VERSION instead of 1
- [x] #3 Doc-comment on WIRE_VERSION: 'breaking wire-format changes bump this; clients can use it to detect daemon protocol version'
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add const WIRE_VERSION: u32 = 1 at module level in whereamid/src/server.rs with doc-comment (AC#3)
2. replace_all v: 1, -> v: WIRE_VERSION, in server.rs (11 production sites; assertion strings use backtick not comma, untouched)
3. Make SUPPORTED_PROTOCOL_VERSION pub in whereami-client/src/lib.rs, extend doc-comment
4. replace_all v: 1, -> whereami_client::SUPPORTED_PROTOCOL_VERSION in main.rs (8 test fixtures) + semantic comment at one site
5. Verify: nix develop -c just e2e, cargo fmt --check, grep confirms sites dropped, protocol tests pass
6. Commit code only, no attribution, do not push, leave In Progress
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented as pragmatic one-const-per-crate (task-0114 shared crate not landed).

- whereamid/src/server.rs: added module-level `const WIRE_VERSION: u32 = 1` with the AC#3 doc-comment; replaced 11 production `v: 1,` sites with `v: WIRE_VERSION,`. The 2 assertion-string literals (~L3218/L3235 describing the contract, backtick form `v: 1`) left verbatim.
- whereami-client/src/lib.rs: made existing `SUPPORTED_PROTOCOL_VERSION` `pub` and extended its doc-comment.
- whereami-client/src/main.rs: replaced 8 #[cfg(test)] fixture `v: 1,` with `v: whereami_client::SUPPORTED_PROTOCOL_VERSION,`; added a semantic comment at one site noting fixtures emit a daemon at the client max version and the daemon emit const is `whereamid::WIRE_VERSION`.

Gate (nix develop -c): just e2e PASS, clippy --all-targets -D warnings clean, cargo fmt --check clean (only benign "Git tree dirty" warning). Full workspace 759 tests, 0 failed. Targeted: protocol_version 2 passed, stats_wire 1 passed. Emitted value unchanged (still 1).

Commit b954e62 (code only; backlog not committed). Left In Progress for review gate; ACs not checked.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Replaced 19 hardcoded `v: 1,` wire-protocol-version literals with named constants (one source of truth per crate; when v=2 happens it is a one-line edit per crate, not a 19-site sed).

Changes (commit b954e62, pure literal->named-constant substitution — emitted value unchanged, still 1):
- whereamid/src/server.rs: new module-level `const WIRE_VERSION: u32 = 1;` with doc-comment (AC#3); 11 production response-construction sites now use `v: WIRE_VERSION`.
- whereami-client: made the existing `SUPPORTED_PROTOCOL_VERSION` pub; 8 #[cfg(test)] fixture sites in main.rs now use `whereami_client::SUPPORTED_PROTOCOL_VERSION` (with an in-code note that fixtures emit "a daemon at the client's max supported version"). The two server test ASSERTION-STRING literals (contract-description text, not construction) were intentionally left verbatim.

SCOPE / 0114 deferral (honest): task-0114 (extract a shared whereami-protocol crate) has NOT landed, so whereamid and whereami-client remain separate crates with no shared module — hence TWO constants, not one global source of truth. The daemon's real emit constant is whereamid::WIRE_VERSION; the client fixtures reference the client's SUPPORTED_PROTOCOL_VERSION. Nothing enforces WIRE_VERSION == SUPPORTED_PROTOCOL_VERSION at compile time until 0114 lands; that full cross-crate unification is already tracked by task-0114. AC#1 explicitly permitted this "shared module if 0114 hasn't landed yet" route.

Verification: orchestrator-verified (pure refactor, no behavior change): diff confirmed to contain only const-substitution + doc/comment lines (no emitted-value change); 0 remaining `v: 1,` construction sites; version-assertion tests pass (whereami-client protocol_version 7/7, whereamid stats_wire 1/1 -> emitted v still 1); just e2e 759 tests 0 failed; clippy -D warnings + fmt clean.
<!-- SECTION:FINAL_SUMMARY:END -->
