---
id: TASK-0092
title: resolve_readonly should not write not_found
status: To Do
assignee: []
created_date: '2026-05-13 21:55'
labels:
  - correctness
  - server
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
resolver.rs::resolve_readonly doc comment says 'ephemeral — does NOT write to aps cache'. That is true about the aps table but misleading: the ChainPolicy has not_found: NotFoundPolicy::PerProvider, so a single WiGLE NotFound is written to the not_found table for not_found_ttl_days (default 30 days). The 'whereami resolve aa:bb:cc:dd:ee:ff' surface is supposed to be 'look this up for me, do not change state'. A user using resolve to probe a BSSID and finding it not in WiGLE now blocks the daemon from re-probing for 30 days — they did not ask for that. Fix: change resolve_readonly's policy to not_found: NotFoundPolicy::Never. Locate and the scan loop already populate not_found proactively; the read-only surface does not need to. Or alternatively: keep the current behavior and update the doc comment to be honest ('ephemeral only applies to positive results'). Pick principle-of-least-surprise: option (a).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 resolve_readonly's ChainPolicy uses NotFoundPolicy::Never
- [ ] #2 Doc comment updated to reflect actual behavior (truly ephemeral now)
- [ ] #3 Test: call resolve_readonly for an unknown BSSID, assert not_found table is unchanged
- [ ] #4 Test: call resolve_readonly for a cached BSSID, assert the result is returned without touching aps or not_found
<!-- AC:END -->
