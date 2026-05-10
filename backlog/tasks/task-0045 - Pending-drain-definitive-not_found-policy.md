---
id: TASK-0045
title: Pending drain definitive not_found policy
status: To Do
assignee: []
created_date: '2026-05-10 05:39'
labels:
  - pending
  - policy
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
drain_once never marks BSSIDs as not_found even after a definitive Apple+WiGLE NotFound; relies on max_attempts to expire stale rows. This costs API quota every drain pass for BSSIDs that genuinely don't exist in any provider. Consider: if all configured providers return NotFound (not RateLimited, not NetworkError), mark the BSSID as not_found and remove from pending. If anyone returns NetworkError, fall back to the current attempts-based policy.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Drain marks not_found when all providers return definitive NotFound for the same BSSID in one pass
- [ ] #2 Drain still uses max_attempts when at least one provider returned NetworkError or RateLimited
- [ ] #3 Test covers both the definitive-NotFound and partial-error cases
<!-- AC:END -->
