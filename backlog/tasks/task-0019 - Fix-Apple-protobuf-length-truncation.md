---
id: TASK-0019
title: Fix Apple protobuf length truncation
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-05-09 21:07'
updated_date: '2026-05-09 22:11'
labels: []
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
apple.rs line 100: proto.len() as u8 silently truncates for payloads >255 bytes. Batch of 15+ BSSIDs produces garbage. Either encode as multi-byte length or validate/cap input size and fail loudly.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 encode_request emits 4-byte big-endian length field, no truncation above 255 bytes
- [x] #2 Unit tests cover N=1..300 BSSIDs and assert length-field invariant
- [x] #3 Existing decode_response unaffected; smoke fuzz of fuzz_apple_decode (~1M execs/30s) shows no new crashes
- [x] #4 No other .len() as u8 truncation sites remain in the codebase
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Confirm wire format: \x00\x00\x00\x01\x00\x00\x00\xLL is two BE u32 fields per arpc.go reference; second u32 is payload length.
2. Replace single byte length write with 4-byte BE u32 of proto.len() (use to_be_bytes on u32).
3. Range-check: bail if proto.len() > u32::MAX (impossible in practice but explicit).
4. Add unit test in apple.rs: encode N=1..300 BSSIDs, parse back the envelope, assert length field == proto.len() and bytes line up.
5. Run cargo build + cargo test + clippy + fmt under nix develop -c.
6. Smoke fuzz fuzz_apple_decode for 30s.
7. Commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Replaced single-byte length write at apple.rs:100 with full 4-byte big-endian u32. Removed the literal \x00\x00\x00 prefix bytes that were the high bytes of the would-be length field.

Wire format confirmed against Go reference (acheong08/apple-corelocation-experiments/lib/arpc.go) which decodes the same envelope as: u16 version, pascal-strings (locale, app id, OS), u32 function id, u32 payload length. The Python references (iSniff-GPS wloc.py and apple_bssid_locator.py) both have the same truncation bug.

Audit: only one .len() as u8 site existed in the tree.

Tests added in apple.rs (4 cases, all green): length field round-trip across N=1..300 BSSIDs, regression assertion that high bytes are non-zero for >255B payloads, protobuf round-trip through the local decoder, empty-input framing.

Fuzzer ran 1.05M iterations in 31s with no crashes (cov 328, ft 1494). Setting LD_LIBRARY_PATH to the 64-bit gcc-13 lib was needed because the cargo-fuzz binary depends on libstdc++ which is not on the devShell default LD path. Worth filing a separate task to wire that into the flake.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Fix Apple WPS protobuf length truncation

Replace the single-byte length write in encode_request with a 4-byte big-endian u32, matching Apple's ARPC framing as decoded by the Go reference implementation. The original Python sources both share the same bug; batches above ~15 BSSIDs (>255-byte protobuf) silently truncated.

Changes:
- whereamid/src/apple.rs: encode payload length as u32::to_be_bytes; removed the now-redundant \x00\x00\x00 prefix that was the high bytes of the length field; documented the ARPC layout in comments.
- New unit tests covering N=1..300 BSSIDs, the >255-byte regression case, and a protobuf round-trip.

Verification: cargo build, cargo test (all 29 + 9 proptests pass, 4 new), cargo clippy --all-targets -D warnings, cargo fmt --check, cargo fuzz run fuzz_apple_decode for 30s (1.05M execs, no crashes).

Not fixed here: the cargo-fuzz binary needs LD_LIBRARY_PATH pointed at gcc's libstdc++.so.6 for a clean run inside the nix devShell. Worth a follow-up task on the flake. No runtime impact on the daemon itself.
<!-- SECTION:FINAL_SUMMARY:END -->
