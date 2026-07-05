---
id: TASK-0222
title: >-
  fuzz_apple_encode roundtrip decodes the envelope, not the payload, and asserts
  nothing
status: Done
assignee: []
created_date: '2026-07-02 04:55'
updated_date: '2026-07-05 09:16'
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
- [x] #1 the fuzz target decodes &envelope[PAYLOAD_OFFSET..], not the whole envelope
- [x] #2 at least one structural equality/subset assertion on the decoded result (no unrequested BSSIDs; devices without Location produce no results)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Decode only the protobuf payload at PAYLOAD_OFFSET, never the ARPC envelope header.
2. Replace the discarded decoder result with a structural assertion: encoded request WifiDevice messages have no Location and therefore decode to no ApInfo results; also retain the requested-BSSID subset invariant.
3. Update the fuzz-target invariant documentation to match what is actually proved.
4. Run focused encoder fuzzing, the full fuzz smoke, exact just e2e, then parallel QA and architecture reviews and commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Mandatory qa-test-runner and mped-architect reviews returned GO on frozen diff 57f294f3e3e552caec3594221b182cd99eda0686df5cf4dbe2b16e6908ba21b2. QA exact fuzz smoke passed 1,152,447 total runs across five targets, including 41,166 updated encoder runs plus deterministic empty and 1,024-BSSID-cap inputs. No current-run crash artifacts were created.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Repaired the Apple encoder fuzz invariant: decode only the protobuf payload after the 50-byte request envelope, require structural decoding to succeed, and assert request WifiDevice entries without Location produce no AP observations. Updated the target documentation to state the real property. Full fuzz and E2E gates passed; QA and architecture reviews returned GO.
<!-- SECTION:FINAL_SUMMARY:END -->
