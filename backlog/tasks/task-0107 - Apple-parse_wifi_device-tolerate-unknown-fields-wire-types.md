---
id: TASK-0107
title: 'Apple parse_wifi_device: tolerate unknown fields/wire-types'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:12'
updated_date: '2026-07-08 18:55'
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
- [x] #1 Unit test: hand-built protobuf with the standard lat/lon plus an extra field (e.g. field=3 wire=0 varint=123) — assert decode succeeds and returns the position
- [x] #2 Same test pattern for parse_wifi_device: unknown sub-fields are skipped without erroring
- [x] #3 Doc-comment on parse_location and parse_wifi_device documents 'tolerant of unknown fields per protobuf spec'
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add doc-comment lines to parse_location and parse_wifi_device: 'tolerant of unknown fields (skipped per protobuf spec)'.
2. AC#1: test parse_location tolerates unknown field 3 wire=0 varint appended to a valid lat/lon location msg -> still returns expected (Some,Some).
3. AC#2: test parse_wifi_device skips unknown device-level varint field (5,wire0) AND unknown wire-type-2 field, plus unknown sub-field inside location submessage -> Ok(Some(ApInfo)) with expected position.
4. Verify meaningfulness: each test's message CONTAINS the unknown field; would fail if skip_field bailed.
5. Gate: cargo test -p whereamid apple (lib+bin), just e2e, clippy -D warnings, fmt --check. Run new tests twice.
6. Commit code only, no attribution.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented TEST+DOC only (no parser behavior change) in commit 833674f.

- Doc-comments added to parse_location and parse_wifi_device stating tolerance of unknown fields (skipped per protobuf spec).
- 3 tests added in whereamid/src/apple.rs mod tests:
  - parse_location_tolerates_unknown_field (AC#1): trailing unknown wire-0 varint + unknown wire-2 field.
  - parse_location_tolerates_unknown_field_between_coords (AC#1 variant): unknowns interleaved between lat/lon.
  - parse_wifi_device_tolerates_unknown_fields (AC#2): device-level wire-0 and wire-2 unknowns + unknown sub-field inside Location.

GOTCHA (important): parse_location tolerates an unknown VARINT field via its own `_ => {}` match arm, NOT skip_field. Only non-varint (wire 1/2/5) unknown fields reach skip_field there. My AC#1 tests therefore include a wire-type-2 unknown field so the skip_field path is genuinely exercised. Verified by a mutation probe (skip_field made to bail!) — all 3 new tests fail under it; without the wire-2 field the two parse_location tests would still pass, i.e. would NOT prove the skip path.

Gate (all green, via nix develop -c):
- cargo test -p whereamid apple: lib 15 passed / bin 17 passed (both binaries include all 3 new tests). Ran twice, identical.
- just e2e: lib 197, bin 455, proptests 12, shutdown 4 — 0 failed.
- clippy --all-targets -D warnings: clean (had to reformat an AC#2 doc list that tripped doc_lazy_continuation).
- cargo fmt --check: clean.

Left In Progress; ACs not checked pending review gate.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Pinned and documented the Apple protobuf parsers' tolerance of unknown fields (contract for future Apple wire-format expansion). Test + doc only — NO parser behavior changed (parse_location/parse_wifi_device already skip unknown tags).

Changes (commit 833674f, whereamid/src/apple.rs):
- 3 tests: parse_location_tolerates_unknown_field + parse_location_tolerates_unknown_field_between_coords (unknowns trailing and interleaved between lat/lon), parse_wifi_device_tolerates_unknown_fields (device-level unknown wire-0 + wire-2 fields plus an unknown sub-field inside the embedded Location submessage). All decode to the expected position.
- Doc-comments on parse_location + parse_wifi_device: "tolerant of unknown fields (skipped per protobuf spec)".

RIGOR (implementer mutation-tested): making skip_field bail!() on the unknown tag fails all 3 new tests -> they genuinely exercise the skip path. IMPORTANT SUBTLETY the implementer found: parse_location tolerates an unknown VARINT (wire-0) via its own `_ => {}` match arm, NOT via skip_field — so the task's AC#1 example (field=3 wire=0 varint) ALONE does not exercise skip_field. The AC#1 fixtures were augmented with an unknown wire-type-2 field so the skip_field path is actually covered.

Verification: test+doc-only diff (verified via git — no production code lines changed), so verified by orchestrator re-run rather than a dual-subagent gate: apple lib 15 / bin 17 (stable across 2 runs in both dual-target binaries), clippy -D warnings clean, fmt clean, e2e green (bin 455). No follow-ups.
<!-- SECTION:FINAL_SUMMARY:END -->
