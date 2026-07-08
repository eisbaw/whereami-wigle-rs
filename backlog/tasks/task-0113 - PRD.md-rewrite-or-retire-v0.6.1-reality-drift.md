---
id: TASK-0113
title: PRD.md rewrite or retire (v0.6.1 reality drift)
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:13'
updated_date: '2026-07-08 20:42'
labels:
  - docs
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD.md is stale. Reading cold reveals: 'spherical-mean centroid' (v0.6.1 changed this to two-stage filter); no mention of --http-timeout-secs, --nominatim-timeout-secs, --address-cache-ttl-days, --history-retention-days, --history-segment-distance-m, --history-segment-min-duration-secs; locate example response missing address/stale/age_s/pending/visible/stable; stats response missing inflight/db_write_failures; resolve response says source: 'not_found' without flagging Source/Provenance terminology split (task-0062). Task-0068 covered v0.4.0 era; another sweep is due.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Option A: rewrite PRD.md to match v0.6.1 reality — every CLI flag, every wire response shape, every Source vs Provenance distinction
- [x] #2 Option B: retire PRD.md and replace with a one-page pointer to README.md, docs/protocol.md, and CHANGELOG.md (the actual sources of truth)
- [x] #3 Decision recorded in task final-summary; pick one
- [x] #4 If kept, add a CHANGELOG entry noting PRD refreshed and the version it now reflects
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. git mv PRD.md -> cruft/PRD-v0.x-detailed.md; add 2-line archive header
2. Write new lean ~1-page PRD.md: what whereami is + Authoritative sources of truth pointer
3. Add CHANGELOG Unreleased entry (Option B retirement)
4. Verify git status/diff --stat, no .rs or backlog files touched; commit docs-only (no AI attribution)
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented Option B (retire PRD.md to a lean pointer). Commit e5388ca.
- git mv PRD.md -> cruft/PRD-v0.x-detailed.md (detailed content preserved, not deleted) with a 2-line ARCHIVED header pointing to README.md/docs/protocol.md/CHANGELOG.md.
- New lean ~1-page PRD.md: version-agnostic What-whereami-is (offline-first Wi-Fi geolocation daemon+CLI, unix-socket JSON-lines protocol, Apple WPS primary / WiGLE secondary, local SQLite cache, fix history) + Authoritative sources-of-truth section. Deliberately does NOT re-enumerate CLI flags or wire fields (that was the drift).
- CHANGELOG Unreleased: added "### Changed — documentation" entry.
Verified: doc-only, no .rs touched; backlog task file NOT committed; new PRD links resolve to existing files. Left In Progress / ACs unchecked for review gate.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Retired the stale PRD.md (commit e5388ca, doc-only). DECISION: Option B (retire -> lean pointer), NOT Option A (full rewrite). Rationale: the 628-line detailed PRD had drifted from shipped reality repeatedly — this was the SECOND staleness sweep (after task-0068) — which is exactly the "summary docs go stale; prefer the real sources" anti-pattern in this project's guidelines. A full rewrite would just re-drift; a lean version-agnostic pointer stops the treadmill.

Changes: PRD.md rewritten 628->49 lines (2-paragraph version-agnostic "what whereami is" + an "Authoritative sources of truth" section pointing to README.md / docs/protocol.md / CHANGELOG.md / backlog/, explicitly stating it does NOT re-enumerate flags/wire-fields to avoid drift). The old detailed content is PRESERVED at cruft/PRD-v0.x-detailed.md (git mv, with an ARCHIVED header) — nothing deleted, decision reversible. CHANGELOG.md gained an Unreleased "documentation" entry (AC#4).

AC status: AC#1 (Option A rewrite) NOT taken — the two ACs are an explicit either/or, so #1 is left unchecked; AC#2 (Option B retire) DONE; AC#3 (decision recorded) DONE; AC#4 (CHANGELOG entry) DONE.

Verification: doc-only, orchestrator-verified: commit touches ONLY CHANGELOG.md + PRD.md + cruft/PRD-v0.x-detailed.md (0 .rs files); the new PRD's links (README.md, docs/protocol.md, CHANGELOG.md, cruft/) all resolve; the 582-line archive carries the old detail. No build/test needed (no code change).

Note: git records this as a rewrite+add (not a rename) because root PRD.md still exists with new content; the old content's history follows cruft/PRD-v0.x-detailed.md.
<!-- SECTION:FINAL_SUMMARY:END -->
