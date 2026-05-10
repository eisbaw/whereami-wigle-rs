---
id: TASK-0072
title: Add docs/protocol.md as canonical wire-format reference
status: Done
assignee: []
created_date: '2026-05-10 11:00'
updated_date: '2026-05-10 14:28'
labels:
  - docs
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Daemon protocol is described in two overlapping places (README:52-80 and PRD:49-118) which have diverged: README is shorter and missing fields (address, stale, age_s on locate). debug, version, history commands are not documented at all. Need a single canonical protocol reference. Found in v0.4.0 review (Explore).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 docs/protocol.md documents all 7 commands (locate, resolve, scan, stats, debug, version, history) with full request and response JSON schemas
- [x] #2 Notes the 64KiB request limit, JSON-lines framing, one-shot connection semantics, version field
- [x] #3 README and PRD link to docs/protocol.md instead of describing the protocol redundantly
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Created docs/protocol.md as the canonical wire-format reference. Documents all 7 commands (locate, scan, resolve, stats, debug, version, history) with full request and response JSON schemas. Notes 64KiB limit, JSON-lines framing, one-shot connection semantics, version field, common error responses. Includes minimal client examples for whereami CLI, bash+nc, Python (no deps), Rust (typed lib). README.md and PRD.md link to it; the redundant in-line protocol descriptions in README/PRD are abbreviated to stop drifting.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
docs/protocol.md is now the single canonical wire-format reference with all 7 commands fully specified, error envelope, and client examples in 4 languages. README and PRD link out instead of describing the protocol redundantly.
<!-- SECTION:FINAL_SUMMARY:END -->
