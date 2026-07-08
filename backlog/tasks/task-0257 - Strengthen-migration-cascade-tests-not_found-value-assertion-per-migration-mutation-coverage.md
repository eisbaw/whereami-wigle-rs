---
id: TASK-0257
title: >-
  Strengthen migration-cascade tests: not_found value assertion + per-migration
  mutation coverage
status: To Do
assignee: []
created_date: '2026-07-08 19:34'
labels:
  - tests
  - tech-debt
dependencies:
  - TASK-0151
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Optional follow-up from task-0151 review (mped-architect, both LOW). The historical-shape cascade tests (task-0151, db.rs ~5071-5470) are solid; two optional hardenings: (A) the not_found seeded-row survival check asserts only COUNT(*)==1 by bssid, not column VALUES — a migration that rebuilt not_found preserving the PK but corrupting first_seen/last_seen/checked_at would pass (low risk: no migration currently touches not_found). Query back one timestamp column for symmetry with the aps/pending/last_fix/fixes checks. (B) The "genuinely bites" property was EMPIRICALLY mutation-tested for only migrate_v5_to_v6 (stub->no-op->tests fail); v2_to_v3/v3_to_v4/v4_to_v5/v6_to_v7 bite is confirmed by static inspection (each has a dedicated post-cascade assertion) but not measured. Optionally add a single mutant check per remaining migration to make the guarantee measured rather than argued. Neither blocks; both are test-strengthening only.
<!-- SECTION:DESCRIPTION:END -->
