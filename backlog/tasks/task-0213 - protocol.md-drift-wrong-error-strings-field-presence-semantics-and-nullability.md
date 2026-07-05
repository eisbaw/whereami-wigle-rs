---
id: TASK-0213
title: >-
  protocol.md drift: wrong error strings, field-presence semantics, and
  nullability
status: Done
assignee: []
created_date: '2026-07-02 04:54'
updated_date: '2026-07-05 04:14'
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
- [x] #1 protocol.md error catalogue matches daemon output verbatim for unknown cmd, missing cmd, history errors, and read timeout
- [x] #2 every optional locate/scan/debug field documents whether it is omitted or null and under which condition; address semantics match the cache-miss behavior
- [x] #3 a wire-golden test asserts stale is absent on a fresh fix and pins the unknown-cmd error string
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add daemon-side wire golden coverage for a fresh locate response (stale/address/age_s omitted) and the actual handle_connection unknown-cmd error line.
2. Derive and document exact unknown/missing-cmd, read-timeout, and history input/runtime error strings from the production parser/handlers.
3. Reconcile locate, scan, and debug optional-field presence: distinguish omitted fields from explicit null and explain address cache-miss/background behavior.
4. Document --json byte-identical passthrough, CLI-synthesized transport/non-JSON errors, exit status semantics, malformed-ok:true asymmetry, and SIGPIPE 141.
5. Run the repaired nix-shell E2E gate and focused wire tests, mandatory parallel QA/architecture review, complete the tracker, rerun E2E, and commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
forward-carried from task-0183/0198/0199 (all Done):
- --json is now a BYTE-IDENTICAL passthrough of the daemon response line (not a re-serialized struct). protocol.md should state this explicitly: the CLI parses only `ok` to pick the exit code and does not add/drop/reorder fields.
- New CLI-synthesized shape to document: when the daemon is unreachable OR returns a non-JSON line, `whereami --json` prints exactly {"ok":false,"error":"<message>"} (serde-escaped) to stdout and exits 1. This is the ONLY --json line not produced verbatim by the daemon; note it as a client-side transport-error convention, not part of the wire protocol.
- Exit codes: --json exits 1 on ok:false or transport error, matching human mode; success exits 0.
- task-0199: `whereami` now dies on SIGPIPE (status 141) under early-closed pipes; if protocol.md mentions CLI exit codes, 141 is a new documented value.

forward-carried from 0183 review: also document the passthrough asymmetry - under --json the client validates nothing but the ok field, so a malformed-but-ok:true daemon line passes through with exit 0 (dumb relay by design), while human mode fails its typed parse with exit 1. Belongs next to the synthesized transport-error object documentation.

Forward-carried from task-0194: docs/protocol.md resolve `source` vocabulary was extended to api|cache|not_found|unresolved. New value 'unresolved' = the daemon could not consult a provider this call (deferred to in-flight resolution / provider skipped for creds/quota / transient failure); not_found is now explicitly documented as AUTHORITATIVE absence. Added a versioning note that new enum values in existing string fields are additive (v stays 1). If task-0213 reconciles protocol.md vs code: the daemon Provenance enum (whereamid/src/server.rs ~L237) and client Provenance enum (whereami-client/src/lib.rs ~L89) both now carry Unresolved and serialize to 'unresolved'; the wire_enum_tests in both crates pin this spelling.

Forward-carried from task-0102: docs/protocol.md stats section gained an `orphan_tasks_aborted` field (u64), added exactly in sync with the StatsResponse struct + handle_stats. Keep it in the drift reconciliation. Stats response now has: cached_aps, pending_aps, not_found_aps, db_size_bytes, api_calls_today, inflight, db_write_failures, orphan_tasks_aborted.

Reconciled the canonical daemon wire contract against production handlers and serializers. Added production-path tests proving fresh locate omits address/stale/age_s, scan/debug preserve their explicit-null versus omitted shapes, and real handle_connection emits exact unknown/missing-command response lines including framing. Documented the 64 KiB read-cap nuance, exact fixed errors plus dependency/input-varying parser suffixes, complete history validation/runtime failures, and that human error prose is not a machine code. Documented --json transport trimming, body passthrough, boolean-ok exit semantics, schema/v nonvalidation, CLI-only no-v synthesis, usage/target precedence, and SIGPIPE 141. Separate pre-existing findings (typed StatsResponse missing orphan_tasks_aborted and stale ADR-060 synchronous geocoding claims) will be filed as follow-ups.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Canonical protocol documentation now matches current daemon and official client behavior. Mandatory QA and architecture gates returned GO after closing runtime-history, stale-address, dynamic-example, and usage-precedence review gaps. Final review validation passed 419 shell-based E2E tests with strict clippy; each new production-path wire test passed five repeated executions and diff/format checks were clean.
<!-- SECTION:FINAL_SUMMARY:END -->
