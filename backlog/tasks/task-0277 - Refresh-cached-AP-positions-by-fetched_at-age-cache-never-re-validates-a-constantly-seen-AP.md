---
id: TASK-0277
title: >-
  Refresh cached AP positions by fetched_at age (cache never re-validates a
  constantly-seen AP)
status: To Do
assignee: []
created_date: '2026-10-29T11:49:00Z'
labels:
  - resolver
  - cache
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
aps rows carry first_seen, last_seen and fetched_at, but retention prunes only by last_seen (--aps-retention-days), and every sighting bumps last_seen. An AP seen daily is therefore never re-looked-up: its position lives forever even when the provider has since corrected it. fetched_at is written but never read for any decision.

Observed 2026-10-28: the connected router C4:41:1E:F9:72:BC was cached ~45 km away (old position) while Apple and WiGLE both reported the current one; with one visible AP, locate answered ±100m High at the wrong place.

Goal: stale-by-fetch-age positions are re-validated against providers without dropping the cached position meanwhile (offline answers must keep working). Relates to task-0272 (locally-observed vs provider-reported disagreement); design the two together.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A cached AP whose fetched_at is older than a configurable refresh age is queued for re-lookup while its cached position stays usable
- [ ] #2 A successful re-lookup updates the position and fetched_at; a definitive provider miss is handled per the existing not_found policy without silently keeping a wrong position forever
- [ ] #3 The refresh age is a daemon flag with a Nix module option (flag-parity holds); 0 disables refresh
- [ ] #4 Re-lookups respect existing provider budgets (WiGLE daily limit) and do not starve first-time pending lookups
- [ ] #5 Tests cover: stale entry re-queued, fresh entry not re-queued, last_seen bumps do not reset the refresh clock
<!-- AC:END -->
