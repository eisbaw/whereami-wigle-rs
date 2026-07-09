---
id: TASK-0271
title: Expose RssiSource provenance on debug and wire types
status: Done
assignee:
  - '@myself'
created_date: '2026-07-09 02:56'
updated_date: '2026-07-09 07:58'
labels: []
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Follow-up to task-0126. task-0126 introduced Rssi/RssiSource and threads provenance (iw authoritative dBm vs nmcli biased linearization) in-memory from scanner to PositionedAp, but the wire/debug types NetworkEntry.signal_dbm and DebugBssid.signal_dbm still expose only a bare i32. An operator inspecting scan/debug output cannot see whether an AP dBm is authoritative or a linearized approximation. Consider adding an optional source field to the debug wire so provenance is visible externally. Requires a whereami-protocol change, deliberately out of scope for task-0126 which was daemon-internal only.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Debug/scan wire output surfaces per-AP RSSI provenance (iw vs nmcli-linearized)
- [x] #2 whereami-protocol version and compat handled
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add wire RssiSource enum (Iw/Nmcli/Unknown, tokens iw/nmcli) + Display in whereami-protocol
2. Add optional additive rssi_source field to NetworkEntry + DebugBssid (serde default, skip_if none)
3. From<daemon RssiSource> map in whereamid/rssi.rs
4. Thread provenance through scan handler + debug_inputs tuple in server.rs
5. Client render suffix (iw)/(nmcli~); re-export enum
6. Wire tests: byte-shape present/omitted, variant round-trip+unknown, back-compat fixtures decode None; protocol.md
7. just e2e + cargo fmt --check
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented wire exposure of RSSI provenance (commit a5d878b).
- Added whereami_protocol::RssiSource wire enum (tokens iw/nmcli, serde(other) Unknown) + Display; From<rssi::RssiSource> map in whereamid/rssi.rs (internal enum kept, mapped at boundary).
- Additive optional rssi_source on NetworkEntry + DebugBssid (serde default + skip_serializing_if none); protocol stays v=1.
- Threaded from ScanEntry.rssi_source: scan handler + debug DebugInputRow tuple.
- Client renders (iw)/(nmcli~); None renders empty (layout byte-stable).
- Wire tests: byte-shape present/omitted, variant round-trip + unknown token, back-compat fixtures decode None. protocol.md documented.
- Gate: just e2e = 900 passed (baseline 896 + 4 new), clippy -D warnings clean, cargo fmt --check clean.
- Left In Progress for review; ACs not checked.

## Review gate (orchestrator, independent) — GO/GO
- QA (2x): 900/900, zero flake; clippy 0 -D warnings; fmt clean. Byte-stable tests (network_entry omit, debug_bssid signal-null, scan byte-pin) still pass -> additive proven. New tests: rssi_source variants round-trip + unknown-token tolerance, present/omitted byte-shape, network_entry appends-when-present, client render_scan_shows_rssi_provenance_when_present + render byte-stable.
- Architecture PRIMARY CHECK — provenance is GENUINE, not hollow. Full chain traced: nmcli is PRIMARY backend (iw fallback, scanner.rs:319-325); parse_nmcli_output stamps NmcliPercentLinearized (795-802), parse_iw_output stamps IwDbm (956/1023); preserved through debounce scan_to_sample -> ScanEntry.rssi_source; handle_scan/handle_debug map via From<rssi::RssiSource> (IwDbm->Iw, NmcliPercentLinearized->Nmcli). So a real nmcli AP -> "rssi_source":"nmcli" -> "(nmcli~)"; iw -> "iw" -> "(iw)". The IwDbm literals in trilaterate/debounce/main are the INERT identity re-wrap sites (task-0126 review), NOT the scan/debug wire path.
- CONFIRMED: (1) additive/byte-identical when None (skip_serializing_if, v=1); (2) back-compat both ways (no deny_unknown_fields; old fixture->None; unknown token->Unknown via serde(other)); (3) From map correct + daemon never constructs wire Unknown (strict-by-construction like Provenance/DbStatus); (4) threaded from entry.rssi_source, not hardcoded (DebugInputRow widened; scanned->Some, stable-absent->None paired with null signal); (5) render None->"" byte-identical; zero trilateration/weight change.
- LOW (accepted, cosmetic): the render arm for wire Unknown -> "(?)" has no dedicated render test (only reachable when a FUTURE daemon emits an unknown token; the decode-side tolerance IS tested).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Exposed RSSI provenance (iw-authoritative vs nmcli-linearized) on the debug/scan wire, completing the operator-visible half of task-0126 (which threaded it in-memory only).

- New wire enum whereami_protocol::RssiSource { Iw, Nmcli, #[serde(other)] Unknown } (tokens "iw"/"nmcli", Display), + From<rssi::RssiSource> map at the daemon boundary. Wire-enum + From approach chosen over moving the daemon-internal enum (used at ~30 sites).
- Additive #[serde(default, skip_serializing_if="none")] rssi_source: Option<RssiSource> on DebugBssid AND NetworkEntry — byte-identical when None (protocol stays v=1), back-compat both ways (old daemon omits -> None; new key ignored by old client; unknown token -> Unknown).
- Threaded genuinely from the live scan sample (ScanEntry.rssi_source): a real nmcli-sourced AP surfaces "nmcli"/(nmcli~), iw surfaces "iw"/(iw) — reviewer-verified end-to-end (nmcli is the primary backend, so provenance is genuinely differentiated, not hollow). debug_inputs widened to a named DebugInputRow carrying Option<RssiSource>.
- Client render appends (iw)/(nmcli~)/(?)/nothing after the dBm; None -> empty -> byte-identical layout.
- wire.rs tests (byte-shape present/omitted, variant round-trip, unknown tolerance, back-compat->None) + protocol.md.

No trilateration/weight change (task-0126 path untouched). 900 workspace tests, dual gate GO/GO. Commit a5d878bf.
<!-- SECTION:FINAL_SUMMARY:END -->
