---
id: TASK-0088
title: 'WiGLE quota: do not refund on JSON parse failure'
status: To Do
assignee: []
created_date: '2026-05-13 21:54'
labels:
  - correctness
  - wigle
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
provider.rs::wigle_lookup currently refunds the local API quota slot for any WigleError::Network. wigle.rs returns Network for: (a) reqwest send error (no upstream quota consumed — refund correct), (b) HTTP 403 (no quota — refund correct), (c) non-2xx other than 429 (uncertain), (d) JSON parse failure of a 200 response (quota WAS consumed by upstream — refund WRONG). Over weeks a daemon running against flaky WiGLE responses gradually under-counts api_calls_today vs upstream reality, eventually getting surprised by upstream 429 long before the local 100-call limit triggers. Low probability in practice (WiGLE JSON shape is stable) but real. Fix: distinguish parse errors from connect/transport errors. Add WigleError::ParseError that maps to a ProviderOutcome variant that does NOT refund the quota slot. The local counter should track WHAT WE BELIEVE UPSTREAM CHARGED — refunds are appropriate only when we are certain upstream did not consume a slot.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 WigleError gains a ParseError variant (or equivalent) distinct from Network
- [ ] #2 wigle.rs returns ParseError when serde_json fails on a 2xx response body
- [ ] #3 provider.rs::wigle_lookup does NOT refund the api_calls_today slot on ParseError; comment explains why
- [ ] #4 Test: simulate a 200 response with malformed JSON; assert api_calls_today is incremented and NOT refunded
- [ ] #5 Test: simulate a connect error (reqwest::Error); assert the slot IS refunded as before
<!-- AC:END -->
