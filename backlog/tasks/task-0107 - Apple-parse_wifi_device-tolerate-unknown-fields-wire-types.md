---
id: TASK-0107
title: 'Apple parse_wifi_device: tolerate unknown fields/wire-types'
status: To Do
assignee: []
created_date: '2026-05-20 21:12'
labels:
  - robustness
  - apple
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/apple.rs:245-268 (parse_location). Unknown wire types fall into 'else { pos = skip_field(...) }' which is correct, but if the response ever includes field=3 with wire=0 (a varint accuracy field that Apple's real wire format does use), it's silently dropped. The decoder never reads accuracy — fine — but a test asserting 'unknown fields are tolerated' would pin this contract for future Apple protocol expansion.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Unit test: hand-built protobuf with the standard lat/lon plus an extra field (e.g. field=3 wire=0 varint=123) — assert decode succeeds and returns the position
- [ ] #2 Same test pattern for parse_wifi_device: unknown sub-fields are skipped without erroring
- [ ] #3 Doc-comment on parse_location and parse_wifi_device documents 'tolerant of unknown fields per protobuf spec'
<!-- AC:END -->
