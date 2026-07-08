---
id: TASK-0263
title: Persisted antipodal-fallback fix loses the sentinel on later stale reads
status: To Do
assignee: []
created_date: '2026-07-08 21:36'
labels:
  - history
  - wire-format
dependencies:
  - TASK-0127
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
From task-0127 review (mped LOW-2). task-0127 made the fresh locate response distinguish the antipodal-fallback (accuracy_m=1000 sentinel) via a response-only fallback bool. But when a CurrentTrilat fallback position WINS and is persisted, new_fix stores only accuracy_m=1000 into LastFix (which has no fallback field). Served LATER as a stale last-fix, it reports accuracy_m~=1000 (+drift), fallback=false — re-introducing the exact in-band ambiguity 0127 removed, but on the STALE path. Out of 0127 AC-5 scope; the accuracy_m=1000 persistence is pre-existing, and stale fixes already carry stale=true + drift-inflated accuracy, so it is partially mitigated. Fix direction: either do NOT persist an antipodal-fallback fix as an authoritative last_fix, OR add a persisted fallback marker to LastFix so the stale path can echo it. LOW.
<!-- SECTION:DESCRIPTION:END -->
