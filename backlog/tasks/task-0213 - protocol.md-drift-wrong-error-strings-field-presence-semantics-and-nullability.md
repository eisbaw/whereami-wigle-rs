---
id: TASK-0213
title: >-
  protocol.md drift: wrong error strings, field-presence semantics, and
  nullability
status: To Do
assignee: []
created_date: '2026-07-02 04:54'
labels:
  - docs
  - protocol
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Multiple doc-vs-code contradictions in the canonical wire reference: (1) documents 'unknown command: <cmd>' error the daemon never emits - the tagged-enum parse yields 'invalid request: unknown variant ...' and 'invalid request: missing field cmd'. (2) locate example shows literal 'stale': false but stale is skip_serializing_if so it is absent when false; 'address present iff --address-approx' is false - address is None on every cache miss even with the flag (background lookup). (3) scan/debug Option fields (ssid null, channel null, scan_age_ms literal null vs scanned_at omitted) undocumented, two absence conventions in one response. (4) missing error strings: history provide range/from-to, invalid from/to timestamp, read timeout. Ref docs/protocol.md:47-51,57-66,119-133,186-196 vs whereamid/src/server.rs:165-217,259-275,362-364,793-803,923-940.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 protocol.md error catalogue matches daemon output verbatim for unknown cmd, missing cmd, history errors, and read timeout
- [ ] #2 every optional locate/scan/debug field documents whether it is omitted or null and under which condition; address semantics match the cache-miss behavior
- [ ] #3 a wire-golden test asserts stale is absent on a fresh fix and pins the unknown-cmd error string
<!-- AC:END -->
