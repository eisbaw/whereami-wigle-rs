---
id: TASK-0215
title: >-
  Add whereami resolve subcommand (documented as official but missing from the
  CLI)
status: To Do
assignee: []
created_date: '2026-07-02 04:54'
labels:
  - cli
  - protocol
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
protocol.md documents resolve as one of seven commands and calls whereami the official client; the library ships WhereAmIClient::resolve(). The CLI Command enum has no Resolve variant, and ResolveResponse is the only response type without Serialize (can't pass through dispatch). The only ways to resolve a BSSID are raw nc/socat or writing Rust. The primary debugging question - where does the daemon think this AP is, and from which provenance - is unavailable in the shipped UX. Ref whereami-client/src/main.rs:59-104, lib.rs:128-136,382-388, docs/protocol.md:69-93.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 whereami resolve AA:BB:CC:DD:EE:FF prints position + provenance, non-zero exit on ok:false
- [ ] #2 --json works for resolve; ResolveResponse derives Serialize
- [ ] #3 README and whereami-client README list the subcommand; >256 BSSIDs fails client-side with the documented limit message
<!-- AC:END -->
