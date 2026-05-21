---
id: TASK-0125
title: 'ApInfo: distinguish provider-reported-position from local-observation'
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
Compass-domain finding #4. ApInfo (db.rs:11-25) presents lat/lon as 'Information about a resolved AP' but those are 'where the provider centroid says this BSSID was last seen by sniffers' — NOT where the AP physically is. Mobile APs (phones, trains, buses) return wildly different coordinates per the provider's last sample. resolver.rs:144 lookup_cached + touch_ap conflates 'we saw this BSSID near us' (updates last_seen) with 'Apple says it's at lat,lon' (sets lat/lon) in the same row.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Rename ApInfo.lat/lon -> provider_reported_lat/provider_reported_lon (OR wrap in struct ProviderReportedPosition { lat, lon, reported_by: Source, reported_at: DateTime })
- [ ] #2 Add aps.locally_observed_lat/lon columns (NULL by default) populated by trilateration consensus
- [ ] #3 Detection: when same BSSID's local-observation centroid disagrees with provider-reported by >1 km repeatedly, mark aps.is_mobile=1 and exclude from trilateration
- [ ] #4 This is the STRUCTURAL principled fix for the mobile-AP issue (task-0093/0094 are heuristic versions)
- [ ] #5 Test: train AP cached at provider-reported (55.74) but locally observed at (55.67) gets is_mobile=1 after N observations
- [ ] #6 Doc-comment on ApInfo names both concepts plainly
<!-- AC:END -->
