---
id: TASK-0215
title: >-
  Add whereami resolve subcommand (documented as official but missing from the
  CLI)
status: To Do
assignee: []
created_date: '2026-07-02 04:54'
updated_date: '2026-07-02 18:19'
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
- [ ] #1 whereami resolve AA:BB:CC:DD:EE:FF prints position + provenance, non-zero exit on ok:false
- [ ] #2 --json works for resolve; ResolveResponse derives Serialize
- [ ] #3 README and whereami-client README list the subcommand; >256 BSSIDs fails client-side with the documented limit message
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
forward-carried from task-0183 (Done): the raw --json passthrough plumbing is your template for a resolve subcommand.
- lib.rs already has resolve_raw(bssids) -> Result<String> (mirrors locate_raw/scan_raw/etc.): it serializes the existing ResolveRequest struct once and calls raw_command(). The typed resolve() delegates through parse_response_line(&self.resolve_raw(...)?). Reuse this — do NOT hand-build request JSON a second time (single source of truth).
- In main.rs, a run_resolve should follow the run_scan/run_locate shape: `if json { emit_json(client.resolve_raw(bssids)); }` for the passthrough, and a pure render_resolve(&ResolveResponse)->String for the human path.
- IMPORTANT (task-0198): sanitize() every daemon-sourced string you print in the human renderer — for resolve that means the ssid and bssid fields of each ResolveResultEntry. The --json path must stay untouched (byte-identical).

Forward-carried from task-0194: the client Provenance enum (whereami-client/src/lib.rs) now has an Unresolved variant (Display -> "unresolved") alongside Api/Cache/NotFound/Unknown. When implementing the `whereami resolve` subcommand, render Unresolved DISTINCTLY from NotFound: unresolved = absence NOT established (deferred / WiGLE unconfigured / quota-exhausted / transient), user should retry; NotFound = authoritative absence. Do not print unresolved as "not found" — that reintroduces the exact client-side blacklisting bug task-0194 fixed on the wire.
<!-- SECTION:NOTES:END -->
