---
id: TASK-0160
title: 'Test: try_reserve_api_call date-rollover (timezone safety)'
status: To Do
assignee: []
created_date: '2026-05-20 21:31'
labels:
  - tests
  - wigle
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Scout P1-6. db.rs:665. The reset logic at lines 670-693 runs when stored_date \!= today. Test at :1241 never advances the clock; the rollover path is dead-as-tested. Replacing Utc::now().format('%Y-%m-%d') with local_now() would silently reset counters at random local times for users in negative-UTC timezones.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Test: manually insert metadata.api_calls_date='1999-01-01', api_calls_today='99'
- [ ] #2 Call try_reserve_api_call(100); assert returned true, api_calls_today == 1 after, api_calls_date is today (UTC)
- [ ] #3 Test that an explicit UTC date is used regardless of TZ env var (set TZ=America/Los_Angeles inside the test for the duration if portable)
<!-- AC:END -->
