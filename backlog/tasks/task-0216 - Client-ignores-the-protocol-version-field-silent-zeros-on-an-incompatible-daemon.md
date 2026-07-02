---
id: TASK-0216
title: >-
  Client ignores the protocol version field -> silent zeros on an incompatible
  daemon
status: To Do
assignee: []
created_date: '2026-07-02 04:54'
labels:
  - protocol
  - cli
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
protocol.md says the version field v is integer-bumped on incompatible changes, but the client deserializes v:u32 on every struct and never compares it. Because every payload field is #[serde(default)], a hypothetical v:2 response with renamed fields still parses 'successfully' and whereami locate prints 0.000000, 0.000000 +-0m (0 sources) rather than failing. The one failure mode versioning exists to prevent is exactly what happens. Ref whereami-client/src/lib.rs:48-78, docs/protocol.md:234-237. Complements task-0145 (daemon-side constant).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 client rejects responses with v above its supported version with an actionable message and non-zero exit
- [ ] #2 protocol.md tells client authors what to do on unknown v
- [ ] #3 a test feeds a v:2 response and asserts the error path, not zeros
<!-- AC:END -->
