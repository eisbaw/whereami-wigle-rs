---
id: TASK-0072
title: Add docs/protocol.md as canonical wire-format reference
status: To Do
assignee: []
created_date: '2026-05-10 11:00'
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
- [ ] #1 docs/protocol.md documents all 7 commands (locate, resolve, scan, stats, debug, version, history) with full request and response JSON schemas
- [ ] #2 Notes the 64KiB request limit, JSON-lines framing, one-shot connection semantics, version field
- [ ] #3 README and PRD link to docs/protocol.md instead of describing the protocol redundantly
<!-- AC:END -->
