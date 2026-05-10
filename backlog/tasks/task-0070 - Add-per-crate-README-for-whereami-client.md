---
id: TASK-0070
title: Add per-crate README for whereami-client
status: To Do
assignee: []
created_date: '2026-05-10 10:59'
labels:
  - docs
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
whereami-client is a publishable Rust library with a typed protocol client (LocateResponse, ResolveResponse, HistoryResponse, etc.) but has no README of its own. External users of the lib have to read the daemon's README to find out the lib exists. Found in v0.4.0 review (Explore).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 whereami-client/README.md describes the public API and shows typed-call examples (client.locate(), client.history(Some('7d'.into()), None, None))
- [ ] #2 Cargo.toml has 'readme = "README.md"' so cargo publish picks it up
<!-- AC:END -->
