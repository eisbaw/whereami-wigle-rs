---
id: TASK-0214
title: >-
  CHANGELOG fabrications: nonexistent resolve subcommand, wrong db_status vocab,
  0.4.0/0.5.0 double-claims
status: To Do
assignee: []
created_date: '2026-07-02 04:54'
labels:
  - docs
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
CHANGELOG.md content errors: (a) 0.4.0 lists a 'resolve' whereami CLI subcommand - no revision of whereami-client ever had it. (b) 0.6.0 claims db_status vocabulary fresh/stale/not_found/unknown; the actual wire vocabulary is cached/pending/not_found/new. (c) 0.4.0 and 0.5.0 (same date) both claim protocol.md, geo module, cooperative shutdown, stats inflight/db_write_failures, parse_range UTF-8 fix, MAX_RESOLVE_BSSIDS, tagged Request enum, and the -90 dBm removal. The changelog is the semver audit trail. Ref CHANGELOG.md:255-257,92-95 and the 0.4.0/0.5.0 sections.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 no CHANGELOG entry names a CLI subcommand that never existed
- [ ] #2 0.6.0's enum entry quotes the real wire strings cached/pending/not_found/new
- [ ] #3 each 0.4.0/0.5.0 item appears in exactly one release, spot-verified against git log
<!-- AC:END -->
