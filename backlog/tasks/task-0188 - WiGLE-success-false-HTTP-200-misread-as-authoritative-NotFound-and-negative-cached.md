---
id: TASK-0188
title: >-
  WiGLE success:false (HTTP 200) misread as authoritative NotFound and
  negative-cached
status: Done
assignee:
  - '@claude'
created_date: '2026-07-02 04:50'
updated_date: '2026-07-02 05:41'
labels:
  - correctness
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
WigleSearchResponse deserializes success:bool but it is #[allow(dead_code)] and never checked; the code returns NotFound whenever total_results==0 || results.is_empty(). WiGLE signals query errors and (historically) quota exhaustion as HTTP 200 {success:false, empty results}. Only 429/403 are special-cased, so a transient API failure -> NotFound -> insert_not_found(bssid) -> BSSID skipped for not_found_ttl_days though it was never actually looked up; the reserved quota slot is consumed with no refund. Ref whereamid/src/wigle.rs:37-46,127-137, provider.rs:141-143, resolver.rs:286-291.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 a 200 response with success:false never produces WigleError::NotFound and never writes the not_found table
- [x] #2 quota-style success:false maps to RateLimited (slot refunded, chain HardStop)
- [x] #3 unit test with a canned success:false body pins the behavior
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add message: Option<String> (serde default) to WigleSearchResponse; remove #[allow(dead_code)] (all fields now read).
2. Extract pure interpret_response(body, bssid) -> Result<ApInfo, WigleError>: if !success => quota-style msg => RateLimited else Network (never NotFound); success:true + zero results/missing trilat => NotFound; else Found.
3. Add looks_like_quota(msg) heuristic (too many queries / quota / rate limit, case-insensitive) with safe fallback to Network.
4. lookup_bssid calls interpret_response after json parse.
5. Tests: success:false quota=>RateLimited; success:false generic=>Network(not NotFound); success:true zero=>NotFound; success:true w/ result=>Found. AC#2 refund+HardStop inherited from existing provider.rs RateLimited arm (unchanged).
6. Gate: nix develop -c just e2e + just fmt.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented in whereamid/src/wigle.rs:
- Added message: Option<String> (serde default) to WigleSearchResponse and removed #[allow(dead_code)] (success + message + results + total_results are all now read).
- Extracted pure interpret_response(body, bssid) -> Result<ApInfo, WigleError> from lookup_bssid so the policy is unit-testable without network.
- success:false now branches BEFORE the zero-results check: quota-style message => WigleError::RateLimited; any other message (or none) => WigleError::Network. Never NotFound.
- looks_like_quota(): case-insensitive substring match on "too many queries" | "quota" | "rate limit". Confirmed via WiGLE forums the real quota body is {"success":false,"message":"too many queries today"}.

AC#1 "never writes not_found": in resolver.rs only ProviderOutcome::NotFound triggers insert_not_found. success:false now yields RateLimited (=>HardStop) or Network (=>NetworkError) in provider.rs, neither of which calls insert_not_found. So the not_found table is never written for success:false.
AC#2 refund + HardStop: inherited from the EXISTING provider.rs RateLimited arm (refund_api_call + ProviderOutcome::HardStop) — unchanged; my change only routes quota-style success:false into it.

Tests (canned JSON bodies via serde_json::from_str):
- success_false_quota_maps_to_rate_limited (AC#2/#3)
- success_false_generic_is_not_not_found (AC#1: Network, not NotFound)
- success_false_no_message_is_not_not_found (AC#1)
- success_true_zero_results_is_not_found (control: genuine NotFound preserved)
- success_true_with_result_is_found (control: happy path)

Gate: nix develop -c just e2e + just fmt green. Tests 199 -> 204 (+5; wigle is only in the bin crate, not the lib, so these are NOT double-counted unlike apple). 0 clippy warnings, fmt clean.

LIMITATION (self-critical): looks_like_quota is a substring heuristic. If WiGLE changes the quota wording, a real quota response would fall through to WigleError::Network instead of RateLimited. This is still SAFE for AC#1 (no NotFound, no negative-cache, slot refunded); the only cost is NetworkError (per-policy retry) instead of HardStop. Verified against the current documented message. Also: modern WiGLE may return quota as HTTP 429 (already handled at the status layer) rather than 200; this change covers the historical/edge 200 path.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Stop WiGLE success:false (HTTP 200) from being misread as an authoritative NotFound and negative-cached (task-0188).

What changed (whereamid/src/wigle.rs):
- The success:bool field was deserialized but never checked (#[allow(dead_code)]); the code returned NotFound whenever results were empty. WiGLE reports query errors and (historically) daily-quota exhaustion as HTTP 200 {"success":false,...}, so a transient failure was written to the not_found table and the BSSID was skipped for not_found_ttl_days without ever being looked up, and the reserved quota slot was consumed with no refund.
- Added a message field, removed the dead_code allow, and extracted a pure interpret_response() that checks success FIRST: quota-style success:false -> WigleError::RateLimited (existing provider.rs arm refunds the slot and HardStops the chain, like HTTP 429); any other success:false -> WigleError::Network (transient). Only success:true with zero results is a genuine NotFound.
- looks_like_quota() classifies the message (case-insensitive substring; confirmed real body is "too many queries today").

Why: never negative-cache a BSSID that was never actually looked up; part of the "never persist garbage from a provider response" theme.

Tests: 5 unit tests over canned JSON bodies (quota=>RateLimited, generic/no-message success:false=>Network not NotFound, success:true zero=>NotFound control, success:true w/ result=>Found control). Workspace 199 -> 204 passing. clippy -D warnings clean, cargo fmt --check clean.

Limitation: quota detection is a substring heuristic; a reworded WiGLE quota message would map to Network (still refunded, still never negative-cached) instead of HardStop. provider.rs was not modified.
<!-- SECTION:FINAL_SUMMARY:END -->
