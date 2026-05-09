---
id: TASK-0020
title: Add HTTP timeouts to all clients
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-05-09 21:07'
updated_date: '2026-05-09 21:49'
labels: []
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
reqwest::Client::new() has no timeout. If Apple/WiGLE/Nominatim hangs, background tasks block forever and leak memory. Build all clients with .timeout(Duration::from_secs(10)).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 All reqwest clients have explicit connect_timeout and total timeout
- [x] #2 Default total timeout is at most 30s
- [x] #3 Build/clippy/test/fmt clean
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add HTTP_CONNECT_TIMEOUT (5s) and HTTP_REQUEST_TIMEOUT (15s) constants in lib.rs (single source of truth)
2. For Nominatim, use a longer total timeout (30s) since OSM is slow
3. Update each client (apple, wigle, beacondb, nominatim) to construct via Client::builder() with .connect_timeout() and .timeout()
4. No new CLI flags - hardcoded constants per MPED simplicity. Add flag later if user pain emerges.
5. Build, clippy, fmt, test
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Created shared whereamid::http module with CONNECT_TIMEOUT=5s, REQUEST_TIMEOUT_FAST=15s, REQUEST_TIMEOUT_NOMINATIM=30s.
Replaced Client::new() in apple, wigle, beacondb, nominatim with client_with_timeout(...).
build/clippy/fmt/test all clean.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Centralized HTTP client construction in `whereamid::http` and made all timeouts explicit.

Changes:
- New module `whereamid/src/http.rs` with shared constants (CONNECT_TIMEOUT 5s, REQUEST_TIMEOUT_FAST 15s, REQUEST_TIMEOUT_NOMINATIM 30s) and a `client_with_timeout(total)` builder.
- apple.rs, wigle.rs, beacondb.rs, nominatim.rs construct via the helper instead of `Client::new()`.
- No new CLI flags: defaults are conservative and uniform; can be parameterized later if a real need emerges.

Why: A bare `reqwest::Client::new()` has no read or connect timeout. A wedged TLS handshake or hung peer would pin background tasks indefinitely and leak Apple/WiGLE lookup futures.

Tests: cargo build, cargo clippy --all-targets -D warnings, cargo fmt --check, cargo test all clean.
<!-- SECTION:FINAL_SUMMARY:END -->
