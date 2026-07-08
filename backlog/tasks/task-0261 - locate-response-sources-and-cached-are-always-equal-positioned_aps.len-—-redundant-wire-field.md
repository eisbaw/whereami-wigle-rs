---
id: TASK-0261
title: >-
  locate response: sources and cached are always equal (positioned_aps.len) —
  redundant wire field?
status: To Do
assignee: []
created_date: '2026-07-08 21:18'
labels:
  - wire-format
  - tech-debt
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Observed during task-0117 review (mped-architect, pre-existing, out of 0117 scope). In handle_locate, both LocateResponse.sources (source_count) and .cached (cached_count) are set to positioned_aps.len() on the PRIMARY path (server.rs ~1961-1962), so `sources == cached` always on a fresh fix (on the stale-fallback ctor cached is 0 and sources is fix.sources). This is the same "redundant/duplicative wire field" class task-0117 addressed (it dropped fetched/pending). Investigate whether `cached` and `sources` are genuinely distinct signals or one is redundant: if they are always equal on every non-stale path, consider dropping one (or renaming to clarify the distinction — e.g. cached = APs that came from cache vs sources = APs that contributed to the fix). AUDIT first (they MIGHT diverge in some path — e.g. if a fix uses fewer sources than cached APs after filtering); only collapse if provably identical. Wire change (client + docs + 0152 fixture) if a field is dropped; backward-compatible via serde(default) like 0117. LOW priority.
<!-- SECTION:DESCRIPTION:END -->
