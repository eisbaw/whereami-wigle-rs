---
id: TASK-0088
title: 'WiGLE quota: do not refund on JSON parse failure'
status: Done
assignee:
  - task0088-implementer
created_date: '2026-05-13 21:54'
updated_date: '2026-07-05 10:23'
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
- [x] #1 WigleError gains a ParseError variant (or equivalent) distinct from Network
- [x] #2 wigle.rs returns ParseError when serde_json fails on a 2xx response body
- [x] #3 provider.rs::wigle_lookup does NOT refund the api_calls_today slot on ParseError; comment explains why
- [x] #4 Test: simulate a 200 response with malformed JSON; assert api_calls_today is incremented and NOT refunded
- [x] #5 Test: simulate a connect error (reqwest::Error); assert the slot IS refunded as before
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Split 2xx JSON decode failures from transport/network failures in WigleError.
2. Preserve a reserved quota lease on parse failure, while refunding only definite pre-response failures.
3. Add provider-level tests using deterministic local HTTP behavior for malformed 200 and connect failure.
4. Run focused tests, formatting, and the required review/E2E gates.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation: added WigleError::ParseError for JSON decode failures after a complete successful HTTP response. provider::wigle_lookup consumes the QuotaLease on ParseError and surfaces a retryable NetworkError; connect/body-transfer failures retain the refundable Network path. Added cfg(test) endpoint injection and provider-level local HTTP tests proving malformed 200 keeps api_calls_today=1 while a reqwest is_connect() failure at 127.0.0.1:0 refunds to 0.

Review history: first frozen hash 8cc7bddd... received QA GO but architecture NO-GO because a bind/drop ephemeral-port fixture had a port-reuse race. Replaced it with deterministic TCP port 0. Corrected frozen hash 93cff6dccd601af0a40101157aaf6d7452d708a9a0fb1677f01b1afe80aae343 received QA GO and architecture GO.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
WiGLE quota accounting now keeps a slot for malformed complete 2xx responses that upstream likely charged, while confirmed connection failures still refund. Deterministic provider-level tests cover both persisted counter outcomes; mandatory QA and architecture reviews passed.
<!-- SECTION:FINAL_SUMMARY:END -->
