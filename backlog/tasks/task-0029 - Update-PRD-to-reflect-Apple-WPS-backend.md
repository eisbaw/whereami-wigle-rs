---
id: TASK-0029
title: Update PRD to reflect Apple WPS backend
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-05-09 21:08'
updated_date: '2026-05-09 22:40'
labels: []
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD still describes WiGLE + BeaconDB as the backends. Actual implementation uses Apple WPS as primary, WiGLE as secondary, BeaconDB is dead code. Update PRD to reflect reality.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Update Architecture diagram and prose to show Apple WPS as primary, WiGLE secondary, BeaconDB inactive.
2. Update Lookup strategy section to reflect resolver.rs::resolve_chain order: Apple -> WiGLE.
3. Note BeaconDB as dead/unused (do not remove the section yet — it documents intent).
4. Add a brief operational notes block: HTTP timeouts configurable, source-priority enforcement on upserts, in-flight dedup of remote lookups.
5. Mark history feature (task-0031) as future work.
6. No code changes; verify daemon source order with grep.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
PRD-only update. Verified ground truth in resolver.rs: resolve_background uses [Provider::Apple, Provider::Wigle] and resolve_readonly uses [Provider::Wigle]. BeaconDB has a client module but is never put into a chain.

Honest correction to the prompt: HTTP timeouts are NOT yet CLI-configurable; they are compile-time constants in http.rs (15s fast, 30s Nominatim, 5s connect). I documented them as "explicit per-client timeouts, currently constants" and noted CLI-configurable as future work, rather than overclaiming.

Updated:
- Architecture diagram and prose: Apple WPS primary, WiGLE secondary, BeaconDB unused.
- locate description: cache-only hot path, never blocks on remote.
- Remote API Backends section rewritten in priority order.
- New Operational notes section: explicit timeouts, source priority, in-flight dedup, schema invariants, decoupled reverse geocoding.
- New Future work section calling out task-0031 location history and other unbuilt items.
- Crate structure block updated to reflect actual files (apple.rs, provider.rs, http.rs, nominatim.rs).
- aps.source comment includes "apple".
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Aligned PRD.md with the implementation as it actually runs.

Key corrections:
- Architecture diagram and lookup-strategy prose now describe Apple WPS as the primary backend and WiGLE as secondary, matching resolver.rs::resolve_background([Apple, Wigle]).
- BeaconDB section explicitly marked as currently unused (the client exists but is not in any provider chain).
- locate is documented as cache-only on the hot path; remote calls are background.
- New Operational notes block documents: explicit per-client HTTP timeouts (currently constants in http.rs, not CLI-tunable — honest about that), source-priority enforcement on upserts, in-flight dedup, schema_version single-row invariant, and the new decoupled reverse-geocoding cache from task-0025.
- New Future work block calls out task-0031 (location history timeseries) as not yet built, plus continuous mode, CLI-configurable timeouts, and active BeaconDB integration.
- Crate structure block updated (apple.rs, provider.rs, http.rs, nominatim.rs).

No code changes; PRD-only edit.
<!-- SECTION:FINAL_SUMMARY:END -->
