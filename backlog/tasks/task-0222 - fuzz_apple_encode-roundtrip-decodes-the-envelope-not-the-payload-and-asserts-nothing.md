---
id: TASK-0222
title: >-
  fuzz_apple_encode roundtrip decodes the envelope, not the payload, and asserts
  nothing
status: To Do
assignee: []
created_date: '2026-07-02 04:55'
labels:
  - testing
  - math
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
fuzz_apple_encode ends with decode_response(&envelope, &bssids) - feeding the full envelope (starting at the version bytes) i.e. garbage protobuf; production strips 10 header bytes and the payload starts at PAYLOAD_OFFSET (a constant the harness itself defines). The doc claims invariant 3 is 'encoded envelope round-trips through decode_response' but no equality is asserted, so the call degrades to a no-panic check on bytes the decoder never sees. The encode->decode BSSID-echo property is only covered by one fixed unit test. Ref whereamid/fuzz/fuzz_targets/fuzz_apple_encode.rs.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 the fuzz target decodes &envelope[PAYLOAD_OFFSET..], not the whole envelope
- [ ] #2 at least one structural equality/subset assertion on the decoded result (no unrequested BSSIDs; devices without Location produce no results)
<!-- AC:END -->
