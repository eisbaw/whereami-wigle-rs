---
id: TASK-0028
title: CLI should use typed response structs
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-05-09 21:07'
updated_date: '2026-05-09 22:37'
labels: []
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
whereami-client main.rs uses raw_command + serde_json::Value for locate and scan, bypassing the typed LocateResponse/ScanResponse structs in lib.rs. If the daemon changes a field name, the compiler wont catch it. Add stale/age_s/scanned_at fields to the typed structs and use them.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Extend lib.rs LocateResponse with stale, age_s; ScanResponse with scan_age_ms, scanned_at; add DebugResponse and VersionResponse types.
2. Replace serde_json::Value indexing in main.rs locate/scan/debug/version with typed deserialization via WhereAmIClient methods.
3. Add typed methods on WhereAmIClient (debug, version) to mirror existing scan/stats.
4. Build / clippy / fmt / test.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Extended LocateResponse with stale + age_s, ScanResponse with scan_age_ms + scanned_at. Added DebugResponse, DebugBssid, VersionResponse. Added typed debug() and version() methods on WhereAmIClient.

Rewrote whereami-client/src/main.rs to use typed responses for locate/scan/stats/debug/version. Removed all serde_json::Value indexing.

Honest behaviour change: --json now serialises the parsed struct instead of forwarding the daemon's raw line. Unknown fields the CLI does not yet model will be dropped. For schema-aware tooling that is desirable; for ad-hoc inspection users may want raw daemon JSON. If that becomes a complaint, expose the raw_command path behind a flag.

All cargo build / test / clippy / fmt clean.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Replaced serde_json::Value indexing in whereami-client/src/main.rs with typed deserialization through the existing WhereAmIClient API.

Lib changes:
- LocateResponse gains stale: bool and age_s: Option<u64> (matching the daemon's wire format).
- ScanResponse gains scan_age_ms and scanned_at.
- New DebugResponse / DebugBssid / VersionResponse types.
- New WhereAmIClient::debug() and ::version() methods.

Main changes:
- Each command path now goes through the typed client; deserialization failures surface as errors rather than silently zero-defaulting.
- --json output now round-trips through the struct (unknown daemon fields are dropped). Documented as a deliberate trade-off in task notes.

Tests: existing suite stays green. No new tests added (this is a refactor; lib structs are exercised by the daemon-side serializers and the existing scan/stats methods already round-trip).

Lint/fmt: cargo clippy --all-targets -D warnings clean, cargo fmt --check clean.
<!-- SECTION:FINAL_SUMMARY:END -->
