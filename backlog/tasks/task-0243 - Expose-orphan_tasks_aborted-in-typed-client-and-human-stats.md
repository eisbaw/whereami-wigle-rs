---
id: TASK-0243
title: Expose orphan_tasks_aborted in typed client and human stats
status: Done
assignee: []
created_date: '2026-07-05 04:15'
updated_date: '2026-07-05 04:30'
labels:
  - client
  - protocol
  - observability
dependencies: []
references:
  - whereami-client/src/lib.rs
  - whereami-client/src/main.rs
  - whereami-client/README.md
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The daemon StatsResponse and canonical protocol include orphan_tasks_aborted, but whereami-client::StatsResponse omits it and the human whereami stats renderer never displays it. Serde silently ignores the extra daemon field, so typed consumers lose shutdown-abort observability while whereami-client/README.md claims response structs are 1:1 wire mirrors. Raw --json preserves the field. Discovered during TASK-0213 review.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 whereami-client StatsResponse decodes orphan_tasks_aborted as u64 with a backward-compatible default for older daemons
- [x] #2 human whereami stats output exposes orphan_tasks_aborted with clear operator-facing wording
- [x] #3 a daemon-shaped stats wire golden proves the typed client preserves every documented stats field, including orphan_tasks_aborted
- [x] #4 whereami-client documentation no longer makes a false 1:1 mirror claim
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add backward-compatible orphan_tasks_aborted decoding to the typed StatsResponse with operator-facing field documentation.
2. Extract the human stats layout into a pure renderer and include the last-clean-shutdown aborted-task count.
3. Add daemon-shaped full-wire and legacy-missing-field decode goldens plus exact human-render output coverage.
4. Correct the client README so typed compatibility behavior and raw --json preservation are described honestly.
5. Run focused tests and repaired nix-shell E2E, mandatory parallel QA/architecture review, complete the tracker, rerun E2E, and commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
TASK-0213 QA and architecture both reproduced this drift. Canonical daemon JSON is already correct; do not remove or rename the wire field.

Implemented typed u64 decoding with a missing-field default, a pure human stats renderer, production-parser current/legacy wire goldens, and accurate typed-versus-raw client documentation. Reviewer findings also root-fixed inaccurate raw-method Rustdocs, a private intra-doc link, and an HTML-like CLI help placeholder. Mandatory QA and architecture re-reviews both returned GO with no findings. Validation: strict Rustdoc with -D warnings passed; just e2e passed 422 tests plus clippy. Release note: adding a public StatsResponse field is source-semver-breaking for exhaustive consumers, so the next pre-1.0 client release should be a minor bump.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Expose orphan_tasks_aborted through the typed client and human stats output, preserve legacy-daemon compatibility, and correct the client API documentation contract.
<!-- SECTION:FINAL_SUMMARY:END -->
