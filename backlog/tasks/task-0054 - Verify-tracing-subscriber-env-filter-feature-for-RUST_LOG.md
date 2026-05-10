---
id: TASK-0054
title: Verify tracing-subscriber env-filter feature for RUST_LOG
status: To Do
assignee: []
created_date: '2026-05-10 10:54'
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
- [ ] #1 Cargo.toml has tracing-subscriber with env-filter feature OR main.rs explicitly constructs an EnvFilter
- [ ] #2 RUST_LOG=debug actually produces debug-level logs in a smoke test
<!-- AC:END -->
