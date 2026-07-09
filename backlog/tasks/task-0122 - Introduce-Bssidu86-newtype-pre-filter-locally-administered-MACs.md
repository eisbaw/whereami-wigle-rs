---
id: TASK-0122
title: 'Introduce Bssid([u8;6]) newtype; pre-filter locally-administered MACs'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:18'
updated_date: '2026-07-09 01:57'
labels:
  - correctness
  - domain
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Compass-domain finding #1. BSSID is a 48-bit IEEE 802 MAC. Its U/L bit (second-least-significant of first octet) being 1 = locally administered = randomized client MAC (Android/iOS hotspots, Linux randomization). The Brazil-in-Copenhagen incident's poisoning AP F6:B1:9C:... has first byte F6 = 1111 0110, U/L bit set — by domain definition it CANNOT be infrastructure. Today scanner.rs:13/293 stores bssid as String and normalize_bssid only uppercases. apple.rs:201 accepts whatever bytes the server sent. This is the PRINCIPLED fix that subsumes task-0093 (SSID denylist): a randomized MAC seen once in Copenhagen, once in Paris, once in Salvador is NOT a moving outlier, it's three different devices wearing the same costume.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Introduce #[repr(transparent)] struct Bssid([u8; 6]) with FromStr / Display / Eq / Hash / Serialize / Deserialize
- [x] #2 Methods: is_locally_administered(&self) -> bool, is_multicast(&self) -> bool, is_broadcast(&self) -> bool
- [x] #3 Parse at the system boundary (scanner.rs nmcli/iw parsers, apple.rs decode_response, server.rs handle_resolve) — reject invalid lengths, non-hex, zeros, broadcast, multicast
- [x] #4 filter_outliers / trilaterate pre-filter: APs whose Bssid.is_locally_administered() are excluded by default
- [x] #5 Test: F6:B1:9C:0A:3A:60 (the Brazilian poisoner) is recognized as locally-administered and excluded from trilateration even before drop_isolated
- [x] #6 Test: well-known BSSIDs (B8:50:01:... TP-Link) are universally-administered and pass
- [x] #7 Note in task notes: this strictly subsumes task-0093 (SSID denylist). 0093 can be closed when this lands.
- [x] #8 resolve rejects any BSSID that is not a well-formed 6-octet MAC (error) before any provider/network call
- [x] #9 a BSSID containing & # ? / or whitespace cannot alter the outbound WiGLE request query parameters (validate at Bssid construction; percent-encode as belt-and-suspenders)
- [x] #10 Locally-administered status is determined at every trilateration/resolve decision point via a deterministic method on the Bssid newtype (compute-on-read); NO denormalized aps.is_locally_admin column is persisted, because it is a pure function of the 6 address bytes and persisting it would violate single-source-of-truth and risk drift (reviewer-confirmed; decision + rationale recorded in notes)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. New whereamid/src/bssid.rs: Bssid([u8;6]) newtype — FromStr (colon/hyphen or bare-12-hex, reject wrong-len/non-hex/zeros/broadcast/multicast), Display (canonical upper colon), Eq/Hash/Ord/Copy, serde as canonical string; methods is_locally_administered/is_multicast/is_broadcast. Register mod in lib.rs (pub) + main.rs.
2. AC#3 boundaries: scanner parse sites via push_validated helper (reject malformed/multicast/broadcast/zeros, KEEP locally-admin); apple decode_response drop decoded bssid that fails Bssid parse; server handle_resolve extract validate_resolve_bssids (reject malformed BEFORE resolver/network = AC#9).
3. AC#4/#6/#7 behavioural filter: in solve_locate_from_cache, before visible_cached/positioned push (hence before drop_isolated), parse ap.bssid -> Bssid; skip locally-administered (debug) and malformed (warn). Excludes F6 poisoner; B8 universal passes.
4. AC#10 WiGLE: lookup_bssid parses bssid->Bssid (reject before HTTP) and builds URL via reqwest::Url::parse_with_params(netid) so & # ? / whitespace can never alter the query.
5. AC#5 = compute-on-read decision: DO NOT add aps.is_locally_admin column (deterministic pure fn of the 6 bytes; persisting = denormalization that can drift). Record rationale in notes.
6. Update trilateration-feeding test fixtures that used locally-administered (02:/aa:) as stand-in infra -> universally-administered MACs (per invariant #3). Add F6 poisoner-excluded + B8 passes + injection-rejected tests.
7. File DEFER follow-up: full String->Bssid propagation across DB rows + whereami-protocol wire types; reference its id in a boundary code comment.
8. Gate: nix develop -c just e2e + cargo fmt --check. Report counts. Leave In Progress.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented (left In Progress for orchestrator dual-review gate).

- New whereamid/src/bssid.rs: Bssid([u8;6]) newtype (repr(transparent)) with FromStr (colon/hyphen/bare-12-hex, byte-wise so no UTF-8 char-boundary panic), Display/Debug (canonical UPPER colon), Eq/Hash/Ord/Copy, serde as canonical string. FromStr rejects wrong-length/non-hex/all-zeros/broadcast/multicast via a typed BssidParseError; semantic checks reuse is_broadcast/is_multicast (single source of truth). Registered mod in lib.rs (pub) + main.rs.
- Boundaries (AC#3): scanner push_validated helper at all 3 parse sites (rejects malformed/multicast/broadcast/zeros, KEEPS locally-admin unicast); apple decode_response drops decoded bssid that fails Bssid parse; server handle_resolve now calls extracted validate_resolve_bssids() which rejects malformed BEFORE resolver/network (AC#9).
- Behavioural filter (AC#4/#6/#7): solve_locate_from_cache excludes locally-administered (debug) and malformed (warn) BSSIDs before visible_cached/positioned push -> before drop_isolated. F6:B1:9C:0A:3A:60 poisoner cluster -> NoPositions; B8:50:01 universal cluster trilaterates.
- WiGLE injection (AC#10): lookup_bssid parses bssid->Bssid (rejects before HTTP) and builds the URL via reqwest::Url::parse_with_params("netid",..) which percent-encodes -> & # ? / whitespace cannot alter the query.

AC#5 DECISION = compute-on-read, NO schema column. is_locally_administered is a deterministic pure function of the first octet (0x02 bit). Persisting aps.is_locally_admin would be denormalization that can drift from the BSSID for zero benefit; no query needs it. Computed on read in the trilateration filter instead. SCHEMA_VERSION stays 8 (the AC said v6, stale).

AC#8: this strictly subsumes task-0093 (SSID denylist) — a randomized MAC is excluded by its address alone, no SSID heuristic. Do NOT close 0093 here (orchestrator will).

DEFERRED follow-up = task-0269: full String->Bssid propagation across db::ApInfo/aps rows and whereami-protocol wire types (ResolveResultEntry/NetworkEntry/DebugBssid). Referenced in a code comment at the solve_locate_from_cache boundary. Still String-typed this cycle: all storage rows + all wire types; boundaries parse-at-use.

GOTCHA / fixture churn: the codebase's test fixtures used locally-administered addresses (02:.. and aa:.. both have the U/L bit set) as stand-in INFRASTRUCTURE. The behavioural filter correctly excludes those, so 31 locate/autolocate tests failed until their trilateration-feeding fixtures were converted to universally-administered MACs (02:00:00:00: -> b8:00:00:00: in server.rs/main.rs; the fixed-home cluster in cached_ineligible_rows_ahead_of_fixed -> b8:). Mobile/non-finite fixtures kept 02: (excluded by earlier guards regardless). scanner iw test 11:22:.. (multicast) -> 10:22:.. Consensus counts (visible/stable) come from the scan sample, not visible_cached, so were unaffected.

Gate: nix develop -c just e2e -> 834 tests passed, 0 failed; clippy --all-targets -D warnings clean; cargo fmt --check clean. No resolve/live-daemon commands run.

## Review gate (orchestrator, independent) — GO/GO + hardening
- QA (2x): 834/834, zero flake; clippy 0 -D warnings; fmt clean. All 7 behavioral/security tests present+green.
- Architecture: verified the 31-fixture conversion is CLEAN — sampled 6 across server.rs, every MAC change is INCIDENTAL to the assertions (no assertion/expected-value/count altered to mask a shift); one (barren 02->00) is a tightening; un-converted 02: literals are all in non-trilateration paths or excluded by earlier is_mobile/non-finite guards (steps 1-2 fire before the LA check step 3). U/L math CONFIRM (0x02 LA, 0x01 multicast, not swapped). Filter placed BEFORE both visible_cached consensus denominator AND solver membership -> poisoner is neither numerator nor denominator; universal MACs unchanged; user-facing visible/stable counts come from the scan sample not visible_cached (verified). Injection CLOSED by construction (Url::parse_with_params from a validated Bssid; no raw netid interpolation remains; FromStr rejects non-hex/len/zeros/broadcast/multicast). Resolve validated before any network call. No String-deferral bypass: every sensitive exit (wigle/trilaterate/scanner/apple) re-validates.
- AC#5 RECONSIDERED (reviewer-confirmed sound): the original "persist aps.is_locally_admin column" was superseded by compute-on-read — is_locally_administered is a total function of the 6 bytes, so a column would violate SSOT and risk drift. SCHEMA_VERSION stays 8. AC#5 rewritten to the outcome (now AC#10) and met.
- HARDENING in-thread (commit 54dcb75): fixed a comment that lied — scanner.rs cited a phantom bssid::is_trilateration_eligible; repointed at the real inline Bssid::is_locally_administered() skip in solve_locate_from_cache.
- Real-world tradeoff (deliberate, documented at debug! + module docs + commit): enterprise virtual/mesh APs using locally-administered MACs are dropped from trilateration — accepted per the poisoning-defense intent.
- Deferred (follow-up task-0269, dep on 0122): full String->Bssid propagation across DB rows + wire types.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Introduced a Bssid([u8;6]) newtype and used it to (1) exclude locally-administered (randomized) MACs from Wi-Fi trilateration and (2) close a WiGLE URL-injection vector — the principled fix that subsumes the task-0093 SSID denylist.

Security/behavioral core:
- New whereamid/src/bssid.rs: #[repr(transparent)] Bssid([u8;6]) with FromStr/Display/Eq/Hash/Ord/Copy/Serialize/Deserialize and is_locally_administered/is_multicast/is_broadcast. FromStr rejects wrong-length/non-hex/all-zeros/broadcast/multicast, so a Bssid can only ever hold 6 hex-derived octets.
- BEHAVIORAL: solve_locate_from_cache excludes is_locally_administered() APs before drop_isolated, so a randomized-MAC poisoning cluster (e.g. the Brazil-in-Copenhagen F6:B1:9C:0A:3A:60) is neither a trilateration input nor a consensus denominator. Universal-administered infra APs (B8:50:01 OUI) trilaterate exactly as before.
- Parse/validate at boundaries: scanner (push_validated), apple decode_response, server handle_resolve (rejects malformed BSSIDs before any provider/network call).
- INJECTION CLOSED: the WiGLE query is built via reqwest::Url::parse_with_params from a validated Bssid (was a raw format!("...netid={}",bssid) interpolation) — & # ? / whitespace can never reach the URL.

Design decision: is_locally_administered is computed on read, NOT persisted as a schema column (it is a pure function of the address bytes; persisting would violate single-source-of-truth). SCHEMA_VERSION unchanged (8).

User impact: randomized-MAC poisoning APs no longer skew locate results; a phone hotspot is still shown in a raw scan but excluded from position math. Legitimate all-infrastructure locates are unchanged. Tradeoff (documented): enterprise virtual/mesh APs that use locally-administered MACs are also excluded from trilateration.

Tests: 834 workspace (+21), two independent gate runs zero flake, clippy 0, fmt clean. Named guards: poisoner_f6_is_locally_administered, locate_kernel_excludes_locally_administered_poisoner_cluster, locate_kernel_keeps_universally_administered_infra, validate_resolve_bssids_rejects_malformed_accepts_wellformed, wigle_query_cannot_be_injection_altered, injection_shaped_strings_never_parse. (31 pre-existing fixtures that used locally-administered stand-in MACs were converted to universal MACs — reviewer-confirmed semantically neutral.)

Scope: this cycle keeps DB rows + wire types as String (parsed at use); full Bssid propagation is follow-up task-0269. Subsumes task-0093 (SSID denylist), now closeable.

Commits: 330ff8e (feature) + 54dcb75 (stale-comment fix).
<!-- SECTION:FINAL_SUMMARY:END -->
