---
id: TASK-0067
title: Add CHANGELOG.md for v0.4.0
status: Done
assignee: []
created_date: '2026-05-10 10:58'
updated_date: '2026-05-10 14:28'
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
- [x] #1 CHANGELOG.md at repo root following Keep-a-Changelog format
- [x] #2 v0.4.0 entry summarizes: provider trait + cascade dedup, atomic rate-limit, in-flight dedup, source-priority schema, single-row schema_version, last_fix persistence, address-cache TTL, configurable HTTP timeouts, location-history feature, antimeridian-safe trilateration, Wi-Fi 6E support, SIGTERM handling, Args parse-time validation, new whereami CLI subcommands (history, version), BeaconDB removal, fuzz harness with cargo-fuzz
- [x] #3 Mentions schema migrations v3/v4/v5 so users know upgrades are forward-only and idempotent
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
CHANGELOG.md created at repo root, Keep-a-Changelog format. v0.4.0 entry covers Apple WPS primary, whereami CLI, location history, last_fix persistence, Provider trait, atomic quota, in-flight dedup, source priority, schema v3->v4->v5 migrations, address-cache TTL, configurable HTTP timeouts, Wi-Fi 6E, SIGTERM drain, Args::validate, stats observability, Justfile + cargo-fuzz harness, docs/protocol.md, whereami-client lib, geo module, home-manager module. Separate sections for Added/Changed/Fixed/Removed/Schema migration/Tests.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
CHANGELOG.md added. v0.4.0 entry summarizes 30+ tasks across Added/Changed/Fixed/Removed plus a schema-migration note for upgraders.
<!-- SECTION:FINAL_SUMMARY:END -->
