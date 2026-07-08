---
id: TASK-0138
title: Document WigleResult._netid leading-underscore intent
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:23'
updated_date: '2026-07-08 19:20'
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
- [x] #1 Replace '_netid: String' with '#[serde(default)] #[allow(dead_code)] netid: Option<String>' OR add a comment '// _ prefix: field must deserialize but is currently unused; remove prefix when wired'
- [x] #2 Apply same pattern to any other _-prefixed serde fields if discovered
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Documented the WigleResult "netid" field (commit; whereamid/src/wigle.rs). Renamed `_netid: Option<String>` (leading-underscore to silence the dead-field warning) to `netid: Option<String>` with an explicit `#[allow(dead_code)]` + comment explaining it is deserialized-but-currently-unused (a documented placeholder for future wiring), which is far clearer than a `_` name that reads as an accidental throwaway. Dropped the now-redundant `#[serde(rename = "netid")]` (field name matches the JSON key). AC#2: swept the whole crate for other `_`-prefixed serde fields — `_netid` was the only one; every other `_`-prefixed item (_guard/_db_runtime/_history_lease/_wipe_admission/_metadata param/_is_cancelled param) is a RAII guard or intentionally-unused parameter, correctly prefixed and NOT a serde field.

Verification: no behavior change (field still unused, JSON still parses). Orchestrator-verified: clippy -D warnings clean (confirms #[allow(dead_code)] is correct and sufficient), whereamid wigle tests 16/16 pass (they parse responses containing "netid" -> confirms deserialization unaffected by the rename), fmt clean.
<!-- SECTION:FINAL_SUMMARY:END -->
