---
id: TASK-0122
title: 'Introduce Bssid([u8;6]) newtype; pre-filter locally-administered MACs'
status: To Do
assignee: []
created_date: '2026-05-20 21:18'
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
- [ ] #1 Introduce #[repr(transparent)] struct Bssid([u8; 6]) with FromStr / Display / Eq / Hash / Serialize / Deserialize
- [ ] #2 Methods: is_locally_administered(&self) -> bool, is_multicast(&self) -> bool, is_broadcast(&self) -> bool
- [ ] #3 Parse at the system boundary (scanner.rs nmcli/iw parsers, apple.rs decode_response, server.rs handle_resolve) — reject invalid lengths, non-hex, zeros, broadcast, multicast
- [ ] #4 filter_outliers / trilaterate pre-filter: APs whose Bssid.is_locally_administered() are excluded by default
- [ ] #5 Schema migration v6: add aps.is_locally_admin BOOL column populated by upsert_ap
- [ ] #6 Test: F6:B1:9C:0A:3A:60 (the Brazilian poisoner) is recognized as locally-administered and excluded from trilateration even before drop_isolated
- [ ] #7 Test: well-known BSSIDs (B8:50:01:... TP-Link) are universally-administered and pass
- [ ] #8 Note in task notes: this strictly subsumes task-0093 (SSID denylist). 0093 can be closed when this lands.
<!-- AC:END -->
