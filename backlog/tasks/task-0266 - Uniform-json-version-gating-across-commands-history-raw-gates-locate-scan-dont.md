---
id: TASK-0266
title: >-
  Uniform --json version-gating across commands (history --raw gates,
  locate/scan don't)
status: Done
assignee:
  - '@myself'
created_date: '2026-07-09 00:34'
updated_date: '2026-07-09 07:43'
labels:
  - refactor
  - wire-format
  - cli
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
task-0133 review LOW-2 (pre-existing, not introduced by 0133). The --json byte-passthrough paths are asymmetric on protocol-version handling: history_fixes_raw runs validate_history_fixes_response -> decode_versioned -> check_version, so a v2 error envelope on 'history --raw --json' synthesizes a client error, whereas 'locate --json'/'scan --json' (which do not validate) pass a v>1 daemon line through byte-identically. Decide the intended contract for --json under an unsupported daemon protocol version and make all commands consistent: either version-gate all --json paths (reject v>SUPPORTED) or none (pure byte passthrough). Currently only the raw-history path gates.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Decision recorded: --json either version-gates uniformly (all commands reject v>SUPPORTED) OR passes through uniformly (no command gates)
- [x] #2 All --json command paths follow the chosen contract; a test pins the behavior for at least one gating and one non-gating command today (locate/scan vs history --raw)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Replace decode_versioned() call in validate_history_fixes_response with plain serde_json::from_str (drops version-gate, keeps capability check).
2. Remove now-dead decode_versioned method + HistoryFixesShape.v field + its ProtocolResponse impl (clippy -D warnings).
3. Add wire tests: v>SUPPORTED history-raw line passes through; capability check (old-daemon segmented) still bites; locate/scan already covered.
4. Update docs/protocol.md --json section: no command version-gates under --json.
5. nix develop -c just e2e + cargo fmt --check.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Decision (AC#1): uniform PASSTHROUGH — no command version-gates under --json

task-0183 defined `--json` as a byte-identical daemon transport (pure passthrough). The TYPED (human/non-json) path already version-gates via parse_response_line -> check_version. So `--json` must NOT reject on protocol version for ANY command. The history-raw path's version-gating (decode_versioned -> check_version) was incidental to its capability check and is removed.

## Implementation
- validate_history_fixes_response: decode_versioned(line) -> plain serde_json::from_str(line). Drops the v>SUPPORTED rejection; KEEPS the capability check (segmented-vs-raw old-daemon detection, which is feature-support, not protocol version).
- Removed now-dead decode_versioned method, HistoryFixesShape.v field, and its ProtocolResponse impl (clippy -D warnings). HistoryFixesShape has no deny_unknown_fields, so a benign additive v2 line still decodes + passes (dropping v does not reduce tolerance vs locate/scan).
- Typed path UNCHANGED: parse_response_line still calls check_version.

## Tests (AC#2)
- raw_history_json_passthrough_accepts_unsupported_protocol_version: v>SUPPORTED raw line validates + passes through byte-identically (formerly rejected).
- raw_history_capability_check_bites_independent_of_protocol_version: a v>SUPPORTED SEGMENTED response STILL bails (capability check intact, version-independent).
- locate/scan already covered by tests/protocol_version.rs::json_locate_passes_newer_protocol_through_byte_identically.

## Docs
- docs/protocol.md --json section: explicit that no daemon-backed command version-gates under --json; history --raw aligned with locate/scan; its client check is a capability check.

## Gate: 896 tests passed (894 baseline + 2 new); clippy -D warnings clean; cargo fmt --check clean.

## Review gate (orchestrator, independent) — GO/GO
- QA (2x): 896/896, zero flake; clippy 0 -D warnings (confirms the removed decode_versioned/HistoryFixesShape.v had no remaining users); fmt clean. All 4 named tests pass incl. the typed-path gate (human_locate_rejects_newer_protocol...).
- Architecture: all 5 CONFIRMED. (1) Uniform passthrough — validate_history_fixes_response swapped decode_versioned -> serde_json::from_str, removing ONLY the check_version rejection; capability check preserved verbatim; test feeds v=SUPPORTED+1 with range+fixes present, asserts no error + byte-identical forward (matches locate/scan). (2) Capability check STILL BITES version-independently — test feeds a segmented ok:true line (no range/fixes) with an unsupported v, asserts the "successful segmented history response" bail (would panic if the check were dropped). (3) Typed path UNCHANGED — parse_response_line still check_version first; the typed version-gate tests are untouched (diff only ADDED 2 history-wire tests). (4) Dead-code removal SAFE — decode_versioned zero remaining refs; HistoryFixesShape.v only fed the deleted debug_assert; the ProtocolResponse trait stays alive (parse_response_line bound + impl_protocol_response! macro for the typed responses). (5) locate/scan untouched; protocol.md accurately states no --json command version-gates (gating lives on the typed path).
- Safety: no meaningful reduction — --json is a byte-passthrough transport (task-0183); a too-new daemon is still rejected the moment a human uses the typed path. The change removes an asymmetry, not a real guard.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Made the --json version contract uniform: NO daemon-backed command version-gates under --json (byte-identical passthrough, per task-0183); protocol-version gating lives only on the typed/human path.

Before, history --raw --json was the lone --json command that rejected a v>SUPPORTED daemon line — because validate_history_fixes_response decoded via decode_versioned (which runs check_version). Swapped that to a plain serde_json::from_str, dropping the version rejection while KEEPING the capability check (an old daemon that ignores raw:true and returns a segmented response -> "upgrade whereamid" bail — a FEATURE check, independent of protocol version). Now history-raw matches locate/scan: a v>SUPPORTED line passes through byte-identically.

Removed the now-dead decode_versioned method, HistoryFixesShape.v field, and its ProtocolResponse impl (clippy -D warnings clean confirms no remaining users). The typed path (parse_response_line -> check_version) is unchanged — a human command still rejects a too-new daemon. protocol.md documents the uniform contract.

Tests: v>SUPPORTED history-raw passes through; the capability check bites regardless of version; locate/scan passthrough + the typed-path gate both still pass. 896 workspace tests, dual gate GO/GO. Commit b58e1ac.
<!-- SECTION:FINAL_SUMMARY:END -->
