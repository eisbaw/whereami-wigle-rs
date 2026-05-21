---
id: TASK-0126
title: Rssi(i32) newtype tracking measurement source (nmcli linear vs iw dBm)
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
Compass-domain finding #5. trilaterate.rs:418 computes weight = 10^(dBm/20). nmcli linear-percent and iw raw-dBm both reach trilaterate as signal_dbm: i32 — the type says they're the same physical quantity but they're not (nmcli's mapping is documented as biased; iw is authoritative). Also weight function has no name — it's not a probability, not a distance, just 'stronger->more vote'. Future contributor will read meaning into 10^(dBm/20) that isn't there.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Introduce #[repr(transparent)] struct Rssi(i32) with Rssi::from_iw_dbm and Rssi::from_nmcli_percent constructors
- [ ] #2 Track source via enum RssiSource { IwDbm, NmcliPercentLinearized } stored alongside (PositionedAp gains a rssi_source field, OR Rssi carries it)
- [ ] #3 Document at the type level: nmcli-derived values may have systematic bias; iw is authoritative; trilateration may down-weight or refuse to mix sources
- [ ] #4 Extract fn vote_weight(rssi: Rssi) -> f64 with a documented name (vote-weight, not 'amplitude' or 'distance proxy')
- [ ] #5 Unit test: Rssi::from_iw_dbm(-60).vote_weight() > Rssi::from_iw_dbm(-80).vote_weight()
<!-- AC:END -->
