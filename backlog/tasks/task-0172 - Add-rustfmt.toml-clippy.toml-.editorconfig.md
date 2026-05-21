---
id: TASK-0172
title: 'Add rustfmt.toml, clippy.toml, .editorconfig'
status: To Do
assignee: []
created_date: '2026-05-20 21:34'
labels:
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Forge F14. Formatting and lint config rely entirely on rustc/cargo defaults. Fine today; will bite the first time someone tries to enforce style on a PR. No unsafe_code lint policy, no clippy::pedantic opt-ins, no documented MSRV in clippy.toml.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Add rustfmt.toml: edition=2021, max_width=100, group_imports=StdExternalCrate, imports_granularity=Module (or chosen defaults documented in task notes)
- [ ] #2 Add clippy.toml: msrv=1.83 (matches task-0164), avoid-breaking-exported-api=true
- [ ] #3 Add .editorconfig: tabs/spaces convention, trim_trailing_whitespace, insert_final_newline
- [ ] #4 Justfile fmt-fix uses the rustfmt.toml (no change needed; cargo fmt picks it up automatically)
<!-- AC:END -->
