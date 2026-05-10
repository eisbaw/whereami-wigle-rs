---
id: TASK-0036
title: Fuzz Apple WPS protobuf encoder path
status: To Do
assignee: []
created_date: '2026-05-10 05:38'
labels:
  - testing
  - fuzz
  - apple
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
task-0019 added unit tests asserting the encoded length field matches payload size for N=1..300 BSSIDs and round-trips through the decoder. The fuzz target fuzz_apple_decode only exercises the decoder against arbitrary bytes. Add a fuzz target that fuzzes encode_request inputs (Vec<[u8;6]> BSSIDs) and asserts: (a) encoder never panics, (b) encoded envelope's length field exactly equals proto.len() as a u32 BE.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 New fuzz target fuzz_apple_encode in whereamid/fuzz/fuzz_targets/
- [ ] #2 Property: roundtrip_decode(encode(bssids)) parses without error
- [ ] #3 Property: u32 BE at length offset == inner protobuf length
- [ ] #4 just fuzz includes the new target
<!-- AC:END -->
