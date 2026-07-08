---
id: TASK-0255
title: >-
  Consolidate the duplicated CapturedEvents/Captured test subscriber impls into
  test_support
status: To Do
assignee: []
created_date: '2026-07-08 18:36'
labels:
  - tech-debt
  - test-infra
dependencies:
  - TASK-0251
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Follow-up from task-0251 review (mped-architect, informational). There are ~3-4 near-identical test-only tracing capture-sink subscribers across the crate — server.rs CapturedEvents (3-field: level/target/message), db.rs CapturedEvents (2-field: level/message), config.rs CapturedEvents (2-field), trilaterate.rs Captured (3-field). They differ only in the captured tuple shape. Now that whereamid/src/test_support.rs exists (task-0251) as a shared lib+bin test-support home, these could be consolidated into one generic capture sink there (e.g. a CaptureLayer capturing (Level, target, message) with helper accessors), removing the duplication. LOW priority / pure test-infra DRY: verify the consolidated sink preserves each call site's existing assertions (target filtering in server.rs vs message-only in db/config). Not blocking anything.
<!-- SECTION:DESCRIPTION:END -->
