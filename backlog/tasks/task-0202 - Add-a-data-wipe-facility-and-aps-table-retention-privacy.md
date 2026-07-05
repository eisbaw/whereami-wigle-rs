---
id: TASK-0202
title: Add a data-wipe facility and aps-table retention (privacy)
status: Done
assignee: []
created_date: '2026-07-02 04:52'
updated_date: '2026-07-05 06:42'
labels:
  - privacy
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
History fixes are pruned by --history-retention-days but there is no user-facing wipe (no subcommand, no just recipe, no documented procedure), and the aps cache (every BSSID/SSID/position ever seen) has no retention/eviction at all. ADR-030 explicitly defers a purge command that was never built. A user needing 'delete everything now' (shared machine, seizure risk) has no supported path. Ref whereamid/src/db.rs, docs/adr/030-cache-first.md:24.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 a single documented command removes all stored location/history/AP rows and the last-known fix (wipe control command + CLI subcommand and/or just recipe)
- [x] #2 aps rows past a configurable age are pruned by the background sweep
- [x] #3 README documents retention defaults and the wipe procedure under a Privacy section
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Ground the privacy contract and tests for a destructive daemon wipe plus configurable AP-row retention, including concurrency, confirmation, and what state is intentionally retained. 2. Add transactional database primitives for wiping privacy-bearing tables and chunk-pruning aps by last_seen, with index/query-plan and boundary tests. 3. Wire a confirmed control-protocol/CLI wipe path that clears persisted location/AP state and relevant in-memory state, and extend the existing background retention sweep with a validated aps retention setting across CLI and Nix modules. 4. Document retention defaults, paths, wipe behavior, and repopulation limits under README Privacy; update canonical protocol/client surfaces and integrated tests. 5. Run focused/stress checks and nix-shell just e2e, require parallel QA/architecture GO, complete the tracker, rerun E2E, and commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Preimplementation contract (QA + architecture, 2026-07-05): implement `whereami wipe --yes`, mapped to a literal server confirmation, and enforce wipe as Unix-socket-only because TCP is unauthenticated. One durable IMMEDIATE transaction deletes aps, pending, not_found, fixes, last_fix and the fixes sqlite_sequence row; preserve schema_version and operational metadata; return exact per-table counts; rollback leaves memory unchanged. A fair privacy epoch/RwLock is the outermost coordination boundary: old scan/resolve/locate/pending/geocode publications are epoch-rejected, wipe holds the exclusive lease through its response-write attempt, and tokenized inflight/geocode claims prevent old-guard ABA after clearing. Clear debouncer, last_fix, address cache, scheduler and inflight state only after DB commit. Collection may repopulate after the response-write attempt terminates by success, failure, timeout, or disconnect. This is logical live-state deletion, not forensic erasure of SQLite/WAL/free pages, snapshots, backups, memory, storage media, logs, or already-issued provider traffic. Add aps retention default 30 days, 0=keep indefinitely, checked maximum/cutoff, indexed 1000-row chunks, immediate then daily round-robin sweep, shared Nix option/flag coverage, ADR-030/protocol/README/PRD/client docs, and deterministic DB/concurrency/transport/CLI tests.

Final-review remediation (2026-07-05): managed memory is cleared under the exclusive wipe lease before the publication epoch advances, so tickets captured anywhere in the commit/clear window remain stale. Wipe admission is single-flight and retained across the durable DB continuation and every response-write outcome; overlapping wipes fail promptly, and the async shutdown wait is bounded without risking a committed-but-uncleared split. Socket access grants destructive wipe authority with no read-only role. A missing complete response leaves completion unknown, so verify or retry the idempotent wipe. Deterministic commit-window, enqueue/cancellation, DB-blocked, response-held, stalled-write, and disconnect regressions cover these guarantees.

Final fail-stop remediation (2026-07-05): the irreversible DB-commit -> managed-clear -> epoch-advance continuation uses a dedicated fail-stop database command. Normal Result errors still return and leave the actor reusable, while a panic terminates the whole process with exit code 70 before stale or partially cleared memory can be served. A subprocess regression injects the panic immediately after commit, proves the child cannot resume, and reopens the file database to verify aps, pending, not_found, fixes, and last_fix are durably empty; a companion test preserves ordinary actor panic recovery.

Fail-stop ordering hardening (2026-07-05): the actual privacy write lease and durable wipe admission are bundled in an outer guard; the commit/clear/epoch transition borrows that guard inside an inner panic catch. Fatal handling therefore begins and remains paused, if necessary, while publication is still barred. The subprocess now prepositions an old-ticket reader at an observed Poll::Pending, pauses fatal handling, proves no reader acquisition, guard drop, or resumed serving, then releases exit 70 and reopens the durably empty database.

QA stress remediation (2026-07-05): after a normal transactional wipe error, the actor explicitly releases its rolled-back durable guard before publishing the error result. This preserves handler-owned response admission while making final permit release deterministic; the focused rollback test passed 50 consecutive runs and the 22-test wipe filter passed 20 consecutive rounds.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Implemented a Unix-socket-only, explicitly confirmed logical location-data wipe spanning the client command/API, daemon protocol, and one atomic SQLite transaction. The wipe reports exact row counts, deletes APs, fixes, pending, not-found, last-fix, and the fixes sequence while retaining schema and non-location metadata. Concurrency uses single-flight admission plus an epoch write barrier held through response delivery; durable actor ownership completes commit, managed-memory clearing, and epoch advancement after cancellation, with fail-stop behavior if cleanup panics after commit. Added resolver/geocode claim tokens to prevent stale ABA removal. Added indexed, chunked AP retention with a 30-day default, 0 meaning forever, bounded Nix/CLI configuration, and alternating startup/daily maintenance. Updated protocol, privacy, deployment, ADR, client, README, changelog, Nix parity/bounds, and VM coverage. Verified 469-test E2E, stress suites, 45 client tests, Nix parity/bounds, and the full Nix VM; final QA and architecture reviews both returned GO with no findings.
<!-- SECTION:FINAL_SUMMARY:END -->
