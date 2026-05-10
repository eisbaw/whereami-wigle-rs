---
id: TASK-0069
title: Refresh docs/architecture.svg or replace with text-art
status: Done
assignee: []
created_date: '2026-05-10 10:59'
updated_date: '2026-05-10 14:28'
labels:
  - docs
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
docs/architecture.svg dated 2026-05-04, predates v0.4.0 features (history pipeline, last_fix persistence, source-priority enforcement, address cache, in-flight dedup). The SVG can drift silently because no one regenerates it. Replace with a text-art diagram in PRD/README (matching the existing PRD style) so it can't drift, OR commit a regeneration recipe to the Justfile. Found in v0.4.0 review (Explore).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Architecture diagram is current as of v0.4.0 (shows history, last_fix persistence, address cache, in-flight dedup)
- [x] #2 Diagram source format makes future regeneration tractable (text-art preferred; if SVG, the source file is committed and a 'just diagram' recipe regenerates it)
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Replaced reliance on docs/architecture.svg with text-art ASCII diagrams in README and PRD that show the current pipeline (Apple WPS primary, WiGLE secondary, Nominatim optional, scan -> debounce -> resolve -> cache -> trilaterate -> last_fix + fixes timeseries). Text-art cannot drift silently — any code change that contradicts it shows up in code review. The legacy SVG remains for now as a static asset; consider removing once readers are guided to the text version.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Architecture diagrams in README and PRD are now ASCII text-art that cannot drift. The stale SVG kept as legacy asset; not authoritative.
<!-- SECTION:FINAL_SUMMARY:END -->
