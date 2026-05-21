---
id: TASK-0155
title: 'Test: HTTP-status mapping for Apple/WiGLE/Nominatim via httpmock'
status: To Do
assignee: []
created_date: '2026-05-20 21:30'
labels:
  - tests
  - providers
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Scout P1-1. apple_lookup, wigle_lookup, nominatim — all swallow non-200 responses with mapping to ProviderOutcome. No test exercises this surface. If apple_lookup's '>= 400' check becomes '>= 500', a 403 would silently be treated as Found with empty response -> cascade -> NotFound -> permanent mark. Different status codes (200, 403, 429, 500, connection refused, slow body) need explicit mapping tests.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Add httpmock to whereamid dev-dependencies
- [ ] #2 Tests spin a local httpmock server and assert ProviderOutcome variants for: 200+empty, 200+valid, 403, 429, 500, connection refused (server not started), slow response (body timeout)
- [ ] #3 Particularly important for WiGLE: RateLimited (429) vs Network (other 4xx/5xx) map to different ProviderOutcome with different cache side effects
- [ ] #4 Apple: 200+empty -> NotFound; 200+valid -> Found; 5xx -> NetworkError; 4xx -> NetworkError (or hard-stop, document choice)
- [ ] #5 Nominatim: 200+valid -> Some(address); 200+empty results array -> None; 4xx/5xx -> error path
<!-- AC:END -->
