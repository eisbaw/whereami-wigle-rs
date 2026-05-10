---
id: TASK-0070
title: Add per-crate README for whereami-client
status: Done
assignee: []
created_date: '2026-05-10 10:59'
updated_date: '2026-05-10 14:28'
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
- [x] #1 whereami-client/README.md describes the public API and shows typed-call examples (client.locate(), client.history(Some('7d'.into()), None, None))
- [x] #2 Cargo.toml has 'readme = "README.md"' so cargo publish picks it up
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Added whereami-client/README.md with library API examples (locate, history, custom address, DaemonResponse trait), CLI usage, link to docs/protocol.md. Added readme + description + license fields to whereami-client/Cargo.toml so cargo publish picks them up.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Per-crate README for whereami-client. Cargo.toml updated with publish metadata (readme, description, license).
<!-- SECTION:FINAL_SUMMARY:END -->
