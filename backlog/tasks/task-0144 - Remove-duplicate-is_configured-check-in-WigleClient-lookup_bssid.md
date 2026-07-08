---
id: TASK-0144
title: 'Remove duplicate is_configured check in WigleClient::lookup_bssid'
status: Done
assignee:
  - '@myself'
created_date: '2026-05-20 21:26'
updated_date: '2026-07-08 18:58'
labels:
  - hygiene
  - refactor
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
src/wigle.rs:88 + src/provider.rs:120. Both gates check is_configured. Provider returns Skipped; WigleClient returns WigleError::Network. Two different outcomes for the same precondition violation — second can only fire if someone bypasses the provider abstraction, which today no one does. Dead defense.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Remove the is_configured check from WigleClient::lookup_bssid (or downgrade to debug_assert!)
- [x] #2 Doc-comment on WigleClient::lookup_bssid: 'caller must have verified is_configured(); use Provider abstraction'
- [x] #3 All existing tests pass
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Removed the dead is_configured re-check in WigleClient::lookup_bssid. The sole production caller (provider::wigle_lookup, provider.rs:120) already gates on is_configured() and returns ProviderOutcome::Skipped before reaching lookup_bssid, so the runtime WigleError::Network branch was unreachable dead defense. Downgraded to debug_assert!(self.is_configured(), ...) (fail-fast in debug/test if the Provider abstraction is ever bypassed; no dead branch in release) and documented the precondition on lookup_bssid (AC#1/#2). No test exercises the unconfigured-lookup path, so nothing relied on the old Err.

Verification: one-line hygiene change to a confirmed-dead branch, verified by orchestrator re-run (not a dual-subagent gate): clippy -D warnings clean (no unused anyhow), fmt clean, FULL workspace suite green (745 tests; the debug_assert never trips, confirming no test reaches lookup_bssid unconfigured). No follow-ups.
<!-- SECTION:FINAL_SUMMARY:END -->
