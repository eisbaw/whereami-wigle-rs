---
id: TASK-0138
title: Document WigleResult._netid leading-underscore intent
status: To Do
assignee: []
created_date: '2026-05-20 21:23'
labels:
  - hygiene
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/wigle.rs:50. The struct uses '_netid' with a leading underscore to silence dead-field warnings while still requiring serde to parse the field. Known Rust pattern but undocumented; a future contributor will see _netid and think it's intentionally unused, fail to wire it through, and never know there was a reason it had to deserialize.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Replace '_netid: String' with '#[serde(default)] #[allow(dead_code)] netid: Option<String>' OR add a comment '// _ prefix: field must deserialize but is currently unused; remove prefix when wired'
- [ ] #2 Apply same pattern to any other _-prefixed serde fields if discovered
<!-- AC:END -->
