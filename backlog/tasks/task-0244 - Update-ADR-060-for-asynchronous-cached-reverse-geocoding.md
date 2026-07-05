---
id: TASK-0244
title: Update ADR-060 for asynchronous cached reverse geocoding
status: Done
assignee: []
created_date: '2026-07-05 04:15'
updated_date: '2026-07-05 04:42'
labels:
  - docs
  - architecture
  - nominatim
dependencies: []
references:
  - docs/adr/060-reverse-geocode-nominatim.md
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
ADR-060 still says --address-approx adds a synchronous 100-300ms Nominatim call to every locate, performs one request per locate, and does no caching. Production now uses a bounded background geocode scheduler plus rounded-cell cache: cache hits may include address immediately, cache misses omit address and schedule work, and persisted stale fixes can carry an address regardless of the current flag. The accepted ADR now contradicts code and canonical protocol. Discovered during TASK-0213 review.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 ADR-060 describes the response-critical cache-hit and background cache-miss behavior accurately
- [x] #2 obsolete synchronous latency, one-call-per-locate, and no-caching claims are removed or explicitly superseded
- [x] #3 privacy/rate-limit consequences describe when Nominatim is contacted and the bounded scheduler behavior
- [x] #4 ADR index/status or amendment metadata makes the updated decision history clear
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Reconcile ADR-060 against the current rounded-cell cache, latest-only bounded scheduler, Nominatim rate gate, and canonical locate field-presence contract. 2. Amend the ADR decision/rationale/consequences so cache hits are response-critical, misses schedule background work, and stale persisted fixes are described accurately. 3. Add amendment metadata and update the ADR index entry so the decision history is explicit. 4. Validate links/formatting, run the full Nix-shell E2E gate, obtain mandatory parallel QA and architecture GO, complete the tracker, rerun E2E, and commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Canonical field-presence behavior is documented in docs/protocol.md by TASK-0213. Keep the ADR consistent with that source and current server geocode scheduler.

Amended ADR-060 in place while preserving the 2026-05-03 provider/opt-in decision and recording the TASK-0025/0037/0046/0193 implementation history. The current decision now documents fresh hit/miss/disabled/stale behavior, process-local 256-cell TTL cache, best-effort last_fix persistence, one-worker one-active/latest-pending scheduling, the current two-second pre-dispatch budget, per-process rate gating, retry/failure semantics, and actual-coordinate privacy exposure. Corrected two matching PRD overclaims about guaranteed next-locate inclusion and guaranteed dispatch. Mandatory QA and architecture final reviews returned GO with no findings. Validation: 17/17 Markdown targets, focused scheduler/cache tests, just fmt, git diff --check, and just e2e with 422 tests plus strict clippy all passed.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Reconciled ADR-060 and related PRD statements with the production asynchronous, bounded, TTL-cached Nominatim design and its persistence, failure, and privacy limits.
<!-- SECTION:FINAL_SUMMARY:END -->
