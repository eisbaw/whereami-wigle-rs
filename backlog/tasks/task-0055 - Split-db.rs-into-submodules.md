---
id: TASK-0055
title: Split db.rs into submodules
status: To Do
assignee: []
created_date: '2026-05-10 10:54'
labels:
  - refactor
  - db
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
db.rs is 1518 lines mixing connection management, migrations, Source enum, CRUD across 6 tables, stats accessors, and 570 lines of inline tests. Adding the next migration or table now means editing one large file with five concerns. Found in v0.4.0 review (keeper-maintainer, mped-architect).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 db/{mod,migrations,source,aps,pending,not_found,rate_limit,last_fix,fixes}.rs (or similar split). One file per table impl block
- [ ] #2 Public API surface unchanged (use db::Database; etc. still works)
- [ ] #3 Tests live next to the code they exercise
<!-- AC:END -->
