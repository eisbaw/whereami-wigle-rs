---
id: TASK-0054
title: Verify tracing-subscriber env-filter feature for RUST_LOG
status: Done
assignee: []
created_date: '2026-05-10 10:54'
updated_date: '2026-05-10 13:22'
labels:
  - observability
  - config
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
main.rs:36 calls tracing_subscriber::fmt::init() which only honours RUST_LOG when the env-filter feature is enabled. Verify Cargo.toml; if missing, debug! is silently dropped at runtime — observability footgun. Found in v0.4.0 review (mped-architect).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Cargo.toml has tracing-subscriber with env-filter feature OR main.rs explicitly constructs an EnvFilter
- [x] #2 RUST_LOG=debug actually produces debug-level logs in a smoke test
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Added env-filter feature to tracing-subscriber in whereamid/Cargo.toml. main.rs now constructs an EnvFilter from RUST_LOG, falling back to 'info' when unset. Verified: RUST_LOG=whereamid=debug actually produces debug! lines now (was silently dropped before).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
RUST_LOG now actually controls log level. tracing-subscriber gained env-filter feature; main.rs constructs an explicit EnvFilter that reads RUST_LOG and falls back to 'info'.
<!-- SECTION:FINAL_SUMMARY:END -->
