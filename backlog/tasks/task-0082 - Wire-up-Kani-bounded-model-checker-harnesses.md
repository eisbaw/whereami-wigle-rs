---
id: TASK-0082
title: Wire up Kani bounded-model-checker harnesses
status: Done
assignee:
  - '@claude'
created_date: '2026-05-10 20:43'
updated_date: '2026-05-10 20:49'
labels:
  - verification
  - testing
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Add #[cfg(kani)]-gated proof harnesses for the parser-heavy and unsafe-adjacent code: Apple WPS protobuf decoder (decode_varint, decode_tag, decode_length_delimited, parse_wifi_device, decode_response), scanner string parsers (freq_to_channel, split_nmcli_fields), history range parser (parse_range), and geo haversine. Kani is not in the nix devshell — installation is out-of-band (cargo install --locked kani-verifier && cargo kani setup). Add a 'just kani' recipe with the setup note. Normal cargo build / test / clippy must remain unaffected.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Five or more #[cfg(kani)] proof harnesses inline in the relevant source files
- [x] #2 Normal cargo build / test --workspace / clippy --all-targets -D warnings remain clean (no Kani dep introduced)
- [x] #3 Justfile gains a 'kani' recipe with documentation that setup is out-of-band
- [x] #4 README has a brief Formal verification subsection pointing at the recipe
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. apple.rs: harnesses for decode_varint, decode_tag, decode_length_delimited, parse_wifi_device, decode_response. Bounded-byte arrays N=16-32. #[kani::unwind(11)] on varint loops.
2. scanner.rs: freq_to_channel (total over i32), split_nmcli_fields (32-byte UTF-8 buffer with assume(from_utf8.is_ok()))
3. history.rs: parse_range (32-byte UTF-8 buffer)
4. geo.rs: haversine_m with finite f64 assumes — flag if Kani can't reason about it
5. Justfile: 'kani' recipe with out-of-band setup note
6. README: brief 'Formal verification (Kani)' subsection
7. Normal build/test/clippy/fmt clean
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Added 11 Kani harnesses across 4 modules:
- apple.rs: decode_varint_no_panic, decode_tag_no_panic, decode_length_delimited_no_panic, skip_field_no_panic, parse_wifi_device_no_panic, decode_response_no_panic_empty_requested (6)
- scanner.rs: freq_to_channel_total, split_nmcli_fields_no_panic, normalize_bssid_no_panic (3)
- history.rs: parse_range_no_panic (1)
- geo.rs: haversine_m_no_panic_on_finite_input (1)

All gated on #[cfg(kani)]. No kani crate added to Cargo.toml — Kani injects it at proof time. Added [lints.rust] check-cfg = ['cfg(kani)'] to whereamid/Cargo.toml so Rust 1.80's strict unexpected_cfgs lint doesn't trip on normal builds.

Justfile: 'kani' recipe with out-of-band setup note (cargo install kani-verifier && cargo kani setup).

README: new 'Formal verification (Kani)' subsection under Development with one-time setup and run commands.

DID NOT run Kani locally — installing it is heavy (forked toolchain), out-of-band by design, and the harnesses can be inspected for correctness from the code. Honest caveat: the haversine_m harness reasons over f64 and likely will not terminate quickly; documented in code and README, with a 'comment it out if it doesn't terminate' instruction. The proptest suite covers the same surface stochastically.

Build verification: cargo build clean, 157/157 tests pass, clippy clean, fmt clean. The cfg(kani) gating works as intended — normal builds are unaffected.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
11 Kani #[cfg(kani)]-gated proof harnesses added across apple.rs (6), scanner.rs (3), history.rs (1), geo.rs (1). Targets the parser-heavy and unsafe-adjacent surface that fuzzing already exercises stochastically. Setup is out-of-band; just kani recipe + README docs cover that. Normal cargo build/test/clippy unaffected via [lints.rust] check-cfg + #[cfg(kani)] gating. Did not actually run Kani locally; user installs it on demand.
<!-- SECTION:FINAL_SUMMARY:END -->
