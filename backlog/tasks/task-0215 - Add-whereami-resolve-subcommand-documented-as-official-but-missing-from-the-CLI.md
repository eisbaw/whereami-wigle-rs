---
id: TASK-0215
title: >-
  Add whereami resolve subcommand (documented as official but missing from the
  CLI)
status: Done
assignee:
  - '@myself'
created_date: '2026-07-02 04:54'
updated_date: '2026-07-09 06:25'
labels:
  - cli
  - protocol
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
protocol.md documents resolve as one of seven commands and calls whereami the official client; the library ships WhereAmIClient::resolve(). The CLI Command enum has no Resolve variant, and ResolveResponse is the only response type without Serialize (can't pass through dispatch). The only ways to resolve a BSSID are raw nc/socat or writing Rust. The primary debugging question - where does the daemon think this AP is, and from which provenance - is unavailable in the shipped UX. Ref whereami-client/src/main.rs:59-104, lib.rs:128-136,382-388, docs/protocol.md:69-93.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 whereami resolve AA:BB:CC:DD:EE:FF prints position + provenance, non-zero exit on ok:false
- [x] #2 --json works for resolve; ResolveResponse derives Serialize
- [x] #3 README and whereami-client README list the subcommand; >256 BSSIDs fails client-side with the documented limit message
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Move MAX_RESOLVE_BSSIDS into whereami-protocol as shared const (single source of truth); reference from daemon server.rs and client.
2. Add Resolve { bssids } variant to Command enum (num_args=1.., required).
3. run_resolve: >256 client-side guard (shared const, daemon-wording message) before any socket; --json -> emit_json(resolve_raw); human -> dispatch(resolve) + render_resolve.
4. render_resolve: positioned -> lat,lon + provenance; None -> not found/unresolved (distinct) + provenance; sanitize bssid+ssid.
5. Non-zero exit on ok:false via DaemonError dispatch path.
6. Docs: top README + client README list resolve; verify ResolveResponse Serialize present (0114).
7. Offline tests: render_resolve variants, >256 guard, parse checks. No live daemon.
8. Gate: nix develop -c just e2e + cargo fmt --check.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
forward-carried from task-0183 (Done): the raw --json passthrough plumbing is your template for a resolve subcommand.
- lib.rs already has resolve_raw(bssids) -> Result<String> (mirrors locate_raw/scan_raw/etc.): it serializes the existing ResolveRequest struct once and calls raw_command(). The typed resolve() delegates through parse_response_line(&self.resolve_raw(...)?). Reuse this — do NOT hand-build request JSON a second time (single source of truth).
- In main.rs, a run_resolve should follow the run_scan/run_locate shape: `if json { emit_json(client.resolve_raw(bssids)); }` for the passthrough, and a pure render_resolve(&ResolveResponse)->String for the human path.
- IMPORTANT (task-0198): sanitize() every daemon-sourced string you print in the human renderer — for resolve that means the ssid and bssid fields of each ResolveResultEntry. The --json path must stay untouched (byte-identical).

Forward-carried from task-0194: the client Provenance enum (whereami-client/src/lib.rs) now has an Unresolved variant (Display -> "unresolved") alongside Api/Cache/NotFound/Unknown. When implementing the `whereami resolve` subcommand, render Unresolved DISTINCTLY from NotFound: unresolved = absence NOT established (deferred / WiGLE unconfigured / quota-exhausted / transient), user should retry; NotFound = authoritative absence. Do not print unresolved as "not found" — that reintroduces the exact client-side blacklisting bug task-0194 fixed on the wire.

Implemented (commit 3663283); left In Progress for review.
- Added Command::Resolve { bssids } (1..=256 positional, alias `r`). Human render: positioned -> "bssid  lat, lon  ssid  (provenance)"; unpositioned -> provenance-chosen status keeping task-0194 distinction ("not found" vs "unresolved (retry later)"). BSSID+SSID sanitized (task-0198).
- --json via resolve_raw()+emit_json (byte-identical passthrough); ok:false exits nonzero via DaemonError dispatch. ResolveResponse Serialize already present (task-0114) — verified, no change.
- >256 fail-fast guard (resolve_limit_error) fires before any socket; wording mirrors daemon. Moved MAX_RESOLVE_BSSIDS into whereami-protocol as single source referenced by daemon + client.
- READMEs (top + whereami-client) list resolve.
- 6 offline tests, none hit a daemon. Gate: cargo fmt --check clean; workspace tests 886 passed (baseline 880 + 6); clippy -D warnings clean (just e2e exit 0). resolve --help renders offline.

## Review gate (orchestrator, independent) — GO/GO
- QA (2x): 886/886 (baseline 880 +6), zero flake; clippy 0 -D warnings; fmt clean. 6 new client tests pass; daemon resolve tests (incl. the >256 rejection) still pass after the const move; grep-confirmed NO new test contacts 127.0.0.1:4747 / opens a socket -> no WiGLE/Apple quota burn.
- Architecture: all 6 CONFIRMED. (1) MAX_RESOLVE_BSSIDS moved to whereami-protocol (pub const=256, doc-comment w/ task-0053 rationale); daemon imports it (server.rs guard unchanged, value 256), client references it; single source, no drift/duplicate. (2) render_resolve preserves the task-0194 NotFound("not found") vs Unresolved("unresolved (retry later)") distinction + a third arm for Provenance::Unknown; bssid/ssid sanitize()d (task-0198); test asserts full-string inequality + the negative "!contains(not found)". (3) --json routes through resolve_raw+emit_json byte-identical (no double-serialize), mirrors other commands. (4) ok:false -> exit 1 via DaemonError path (human) / i32::from(!ok) (json). (5) ALL 6 new tests OFFLINE (grep-verified: no WhereAmIClient constructed, no .resolve() call, no socket). (6) >256 guard fires client-side BEFORE any connect; boundary correct (257 rejected, 256 ok); wording byte-identical to daemon.
- STALE-PREMISE corrected: ResolveResponse already derived Serialize (task-0114), so AC#2 derive was pre-satisfied.
- LOW (accepted): client defers per-BSSID MAC validation to the daemon authority (malformed -> ok:false -> exit 1, documented); tests are unit-level (appropriate given the no-live-daemon safety constraint).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added the missing `whereami resolve <BSSID>...` CLI subcommand — protocol.md documented resolve as an official command and the library shipped resolve(), but the CLI Command enum had no Resolve variant, so the primary debugging question (where does the daemon think this AP is, and from which provenance) was unavailable in the shipped UX.

- Command::Resolve { bssids } (clap, 1+ positional, alias r). render_resolve prints per BSSID: positioned -> coords + ssid + provenance; unpositioned -> "not found" (NotFound, authoritative) vs "unresolved (retry later)" (Unresolved, absence not established) — preserving the task-0194 distinction; bssid/ssid sanitized (task-0198). ok:false -> exit 1 (DaemonError path).
- --json passthrough via resolve_raw + emit_json (byte-identical), like the other commands. ResolveResponse already derived Serialize (task-0114) — AC#2 was pre-satisfied.
- >256 client-side guard: moved MAX_RESOLVE_BSSIDS=256 into whereami-protocol as the shared SSOT const (daemon + client both reference it; daemon behavior unchanged). Oversized requests fail client-side BEFORE any socket connect with the daemon-mirrored message (usage error -> stderr + exit 1, per task-0228).
- READMEs (top-level + whereami-client) list the subcommand.

SAFETY: all 6 new tests are OFFLINE (pure render + parse + limit-guard over constructed fixtures) — none connects to a daemon or burns WiGLE/Apple quota (grep-verified in review).

Tests: 886 workspace (+6), dual gate GO/GO, zero flake. Commit 3663283.
<!-- SECTION:FINAL_SUMMARY:END -->
