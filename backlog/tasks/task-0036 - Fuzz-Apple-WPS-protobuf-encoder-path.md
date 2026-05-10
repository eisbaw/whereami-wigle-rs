---
id: TASK-0036
title: Fuzz Apple WPS protobuf encoder path
status: Done
assignee:
  - '@claude'
created_date: '2026-05-10 05:38'
updated_date: '2026-05-10 07:48'
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
- [x] #1 New fuzz target fuzz_apple_encode in whereamid/fuzz/fuzz_targets/
- [x] #2 Property: roundtrip_decode(encode(bssids)) parses without error
- [x] #3 Property: u32 BE at length offset == inner protobuf length
- [x] #4 just fuzz includes the new target
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add new fuzz target whereamid/fuzz/fuzz_targets/fuzz_apple_encode.rs that constructs Vec of 6-byte BSSIDs (1..N) and calls encode_request, then asserts: encoder doesn't panic, u32 BE at length offset == proto.len()
2. Add target to fuzz/Cargo.toml [[bin]]
3. Add 'just fuzz' includes the new target
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Made apple::encode_request public (was pub(crate)). Added fuzz_apple_encode target that:
- Treats fuzz input as a sequence of 6-byte BSSIDs (capped at 1024 to keep iterations fast)
- Asserts the envelope is at least the fixed-header size (50 bytes)
- Asserts the u32 BE payload_len field at offset 46 exactly equals envelope.len() - 50 — the regression check for task-0019 truncation
- Calls decode_response on the encoded envelope to ensure round-trip parsing doesn't panic

Updated Justfile fuzz/fuzz-all recipes to include the new target, and added fuzz-apple-encode for direct invocation. 30s smoke run: 165,469 runs, no crashes.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added a fuzz_apple_encode target that exercises encode_request with structured BSSID inputs and asserts the u32 BE length-field invariant from task-0019. Round-trips through decode_response. 165k runs/30s smoke clean. Justfile fuzz / fuzz-all recipes include the new target; fuzz-apple-encode added for direct invocation. apple::encode_request is now pub instead of pub(crate).
<!-- SECTION:FINAL_SUMMARY:END -->
