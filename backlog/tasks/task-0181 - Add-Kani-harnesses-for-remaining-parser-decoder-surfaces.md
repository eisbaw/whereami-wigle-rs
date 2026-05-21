---
id: TASK-0181
title: Add Kani harnesses for remaining parser/decoder surfaces
status: To Do
assignee: []
created_date: '2026-05-20 21:36'
labels:
  - tests
  - kani
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Today's Kani harnesses (task-0082) prove panic-freedom for: parse_iw_output, parse_nmcli_fields, normalize_bssid, parse_range, decode_apple, encode_apple, freq_to_channel, trilaterate. Per Scout/Compass review, additional surfaces would benefit: Nominatim JSON decode (parse error class), Bssid::from_str (task-0122), Rssi::from_iw_dbm (task-0126). Kani is currently dropped from CI; this task re-enables a sustainable subset.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Identify Kani-tractable proofs vs ones that diverge under bounded input (current geometric_median Capped class)
- [ ] #2 Add #[cfg(kani)] proof harnesses for the tractable surfaces
- [ ] #3 Reduce N bound where convergence is slow (current N=32 for some harnesses may need to be 16 or 8)
- [ ] #4 Optional: Kani in CI on a weekly schedule (Forge F1 / task-0163), not on every push
- [ ] #5 Document which proofs are NOT included and why in trilaterate.rs / etc. doc-comments
<!-- AC:END -->
