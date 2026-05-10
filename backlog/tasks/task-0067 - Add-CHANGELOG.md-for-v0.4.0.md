---
id: TASK-0067
title: Add CHANGELOG.md for v0.4.0
status: To Do
assignee: []
created_date: '2026-05-10 10:58'
labels:
  - docs
  - release
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
31 commits since v0.3.0 implementing 16 backlog tasks; no CHANGELOG exists. Downstream packagers and users have no concise summary of what changed. Found in v0.4.0 review (Explore).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 CHANGELOG.md at repo root following Keep-a-Changelog format
- [ ] #2 v0.4.0 entry summarizes: provider trait + cascade dedup, atomic rate-limit, in-flight dedup, source-priority schema, single-row schema_version, last_fix persistence, address-cache TTL, configurable HTTP timeouts, location-history feature, antimeridian-safe trilateration, Wi-Fi 6E support, SIGTERM handling, Args parse-time validation, new whereami CLI subcommands (history, version), BeaconDB removal, fuzz harness with cargo-fuzz
- [ ] #3 Mentions schema migrations v3/v4/v5 so users know upgrades are forward-only and idempotent
<!-- AC:END -->
