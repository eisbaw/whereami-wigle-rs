---
id: TASK-0214
title: >-
  CHANGELOG fabrications: nonexistent resolve subcommand, wrong db_status vocab,
  0.4.0/0.5.0 double-claims
status: Done
assignee:
  - '@myself'
created_date: '2026-07-02 04:54'
updated_date: '2026-07-09 05:12'
labels:
  - docs
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
CHANGELOG.md content errors: (a) 0.4.0 lists a 'resolve' whereami CLI subcommand - no revision of whereami-client ever had it. (b) 0.6.0 claims db_status vocabulary fresh/stale/not_found/unknown; the actual wire vocabulary is cached/pending/not_found/new. (c) 0.4.0 and 0.5.0 (same date) both claim protocol.md, geo module, cooperative shutdown, stats inflight/db_write_failures, parse_range UTF-8 fix, MAX_RESOLVE_BSSIDS, tagged Request enum, and the -90 dBm removal. The changelog is the semver audit trail. Ref CHANGELOG.md:255-257,92-95 and the 0.4.0/0.5.0 sections.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 no CHANGELOG entry names a CLI subcommand that never existed
- [x] #2 0.6.0's enum entry quotes the real wire strings cached/pending/not_found/new
- [x] #3 each 0.4.0/0.5.0 item appears in exactly one release, spot-verified against git log
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Verify (a) resolve CLI subcommand never existed: git log -S Command::Resolve (empty), original/peak-handrolled/clap main.rs (no resolve arm). DONE.
2. Verify (b) db_status fresh/stale never shipped: git log -S fresh (none), task-0062 6262f9f introduced cached/pending/not_found/new. DONE.
3. Verify (c) 0.4.0/0.5.0 dup ownership: 1fe45ff filed tasks 0046-0081 AFTER v0.4.0 review; impls are descendants (0.5.0 sweep). Each dup 0.4.0 bullet with a 0.5.0 0046-0081 task-ref belongs to 0.5.0.
4. Edit CHANGELOG.md: (a) drop resolve from 0.4.0 subcommand list; (b) fix 0.6.0 db_status vocab; (c) remove duplicated 0.4.0 bullets, keep in 0.5.0.
5. Sanity: cargo fmt --check + cargo build --workspace; git status shows only CHANGELOG.md.
6. Commit docs: no AI attribution.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Verification + fixes (commit 448ad61, CHANGELOG.md only)

### (a) Phantom `resolve` CLI subcommand — CONFIRMED FABRICATION, removed
- `git log --all -S "Command::Resolve" -- whereami-client/src/main.rs` => EMPTY (never referenced in any revision).
- Original CLI (6fd221c): arms locate/scan/stats/help only. Peak hand-rolled (parent of clap migration 80973df): locate/scan/stats/debug/history/version/help — NO resolve. clap enum Command (current, v0.6.1, v0.6.2 tags): Locate/Scan/Stats/Debug/History/(Wipe)/Version — NO Resolve.
- What DOES exist: daemon `resolve` wire command + client library method `resolve()` (whereami-client/src/lib.rs:549). NOT a `whereami resolve` CLI subcommand.
- Change: dropped `resolve` from the 0.4.0 subcommand list; added a precise parenthetical distinguishing the wire command / library method from the (absent) CLI subcommand.

### (b) db_status wire vocab fresh/stale — CONFIRMED FABRICATION, corrected
- `git log --all -S '"fresh"'` and `-S Fresh -- whereami-protocol whereami-client whereamid` => nothing relevant (only an unrelated "reject stale scans" commit). fresh/stale never a wire token.
- task-0062 introduction commit 6262f9f shipped DbStatus = Cached/Pending/NotFound/New and Provenance = Api/Cache/NotFound from day one (snake_case). Current whereami-protocol/src/lib.rs:346 DbStatus serializes cached/pending/not_found/new; Provenance api/cache/not_found/unresolved (unresolved added later by task-0194, so NOT in the 0.6.0 entry).
- Change: 0.6.0 entry now reads provenance `api`/`cache`/`not_found` and db_status `cached`/`pending`/`not_found`/`new` (was `fresh`/`stale`/`not_found`/`unknown`).

### (c) 0.4.0/0.5.0 double-claims — de-duplicated toward 0.5.0
- Key evidence: commit 1fe45ff "Add backlog tasks 0046-0081 from v0.4.0 four-agent review" filed these tasks AFTER 0.4.0 shipped. `git merge-base --is-ancestor 1fe45ff <impl>` true for the disputed impls (228c615/e8b4851/4fc151d/394cab6 and the Phase 3/5/8 commits 33344a6/df05b6c/915db2a). 0.5.0 header itself = "sweep of tasks 0046-0081 ... 8 phases". So every 0.4.0 bullet whose 0.5.0 twin carries a 0046-0081 task-ref belongs to 0.5.0.
- Removed from 0.4.0 (kept in 0.5.0): cooperative shutdown (0075), stats inflight/db_write_failures (0074/0076), docs/protocol.md (0072), whereamid::geo (0081), Apple decoder cross-check (0049), Nominatim empty display_name (0050), tracing-subscriber env-filter (0054), per-BSSID log demote (0073), tagged Request enum (0057), /resolve 256-BSSID cap = MAX_RESOLVE_BSSIDS (0053), parse_range UTF-8 fix (0048), CLI dispatch<T> (0058), last_fix write race (0046), latent migration-stamping bug (0047), -90 dBm removal (0051), drain_cleanup_after_chain (0052).
- Trimmed the "migrations run in transactions / roll back atomically" claim out of 0.4.0 (Schema v5 bullet + Schema-migration footer); transactionality is task-0047 = 0.5.0, now carrying a forward pointer.
- KEPT in 0.4.0 as genuine (no 0.5.0 0046-0081 twin): Apple WPS primary, CLI binary, location history, last_fix persistence, provider trait + resolve_chain + cascade-dedup, atomic quota, in-flight dedup (distinct from stats inflight), source-priority ladder, schema v5 tables, address-cache TTL + get TTL-on-read, HTTP timeouts, Wi-Fi 6E, Args::validate, Justfile, cargo-fuzz, whereami-client library, homeManagerModules, trilateration spherical mean, Apple protobuf u32 length, nmcli signal skip, is_pending fix (task-0045, in 0.4.0 range), BeaconDbClient/clippy removals.

### Verification gate
- `nix develop -c cargo fmt --check` => exit 0. `nix develop -c cargo build --workspace` => Finished (green). Code untouched.
- Commit 448ad61 contains ONLY CHANGELOG.md; author correct; no AI/Co-Authored trailer.

### Honest limits / not fixed
- nmcli "Changed" bullet in 0.4.0 keeps a trailing clause "Also skipped in trilateration candidate ranking (server.rs cold-start path)" that overlaps the -90 removal (0051, now only in 0.5.0). Left intact: the nmcli-parser skip itself is a distinct change not present in the 0.5.0 list, and mid-sentence surgery risked distorting a genuine claim. Minor residual overlap.
- Test-count sections disagree (0.4.0 "grew ... to 134 at release tip" vs 0.5.0 "134 at tip, up from 130 at start of sweep"). Cannot reconstruct exact per-release counts without 0.4.0/0.5.0 tags (only v0.6.1/v0.6.2 exist). Left as-is, noted — did not guess.
- No v0.4.0/v0.5.0 tags exist; release ownership was determined from task-id ranges + the 1fe45ff filing boundary + ancestry, not tag membership.

## Review gate (orchestrator, independent accuracy review) — GO
- Doc-only change (CHANGELOG.md, 13+/45-); the test-gate is moot (orchestrator confirmed build green + fmt clean + only CHANGELOG.md changed), so gated by an INDEPENDENT accuracy review (git-forensics re-run) rather than QA.
- (a) phantom resolve subcommand: CONFIRMED fabrication — git log --all -S "Command::Resolve" empty repo-wide; clap enum Command = Locate/Scan/Stats/Debug/History/Wipe/Version in current + v0.6.1, never Resolve. Corrected wording (daemon Request::Resolve wire cmd @server.rs:407 + client library resolve() @lib.rs:549, NOT a CLI subcommand) is accurate.
- (b) db_status fresh/stale: CONFIRMED fabrication — task-0062 origin commit 6262f9f shipped DbStatus{Cached,Pending,NotFound,New}+snake_case from day one; fresh/stale never a db_status token (the fabricated list even conflated the internal Unknown escape-hatch into the wire vocab). Corrected to cached/pending/not_found/new + provenance api/cache/not_found; correctly did NOT add unresolved (task-0194, later).
- (c) 0.4.0/0.5.0 dedup: CONFIRMED via timestamp ancestry — the review commit 1fe45ff filed tasks 0046-0081 at 2026-05-10 13:03; every removed 0.4.0 bullet maps to a 0046-0081 task implemented 15:xx-16:xx same day (the 0.5.0 sweep). Spot-checked geo/0081, cooperative-shutdown/0075, MAX_RESOLVE_BSSIDS/0053, parse_range/0048 — all post-review. Each disputed item now in exactly one release. Pre-review items correctly KEPT in 0.4.0 (nmcli signal-skip/0043 @09:00 stayed; -90 dBm/0051 @15:14 moved to 0.5.0).
- No true claim destroyed (genuine 0.4.0 items retained: CLI binary, location history, provider trait/resolve_chain, schema v5, is_pending fix). No correction found inaccurate.
- Honest residuals (accepted): a trailing nmcli clause cosmetic overlap; per-release test-count sections disagree but no v0.4.0/v0.5.0 tags exist to reconstruct exact counts (left, not guessed); one migration-rollback test-highlight arguably 0.5.0 under the same test-attribution limitation.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Corrected three CHANGELOG.md fabrications so the semver audit trail is honest, verifying each against git history + code.

(a) Removed the phantom whereami resolve CLI subcommand from 0.4.0 — git shows no Command::Resolve ever existed; what shipped is the daemon resolve wire command + the client library resolve() method (now precisely worded).
(b) Fixed the 0.6.0 db_status/provenance wire vocabulary from the fabricated fresh/stale/not_found/unknown to the real cached/pending/not_found/new (+ provenance api/cache/not_found); task-0062 shipped these from commit 6262f9f. Did not add unresolved (a later task-0194 addition).
(c) De-duplicated 16 bullets that appeared in BOTH the same-date 0.4.0 and 0.5.0 sections, attributing each to its true release via git ancestry + the 0046-0081 task-refs filed after the v0.4.0 review; each disputed item now appears in exactly one release, with pre-review items correctly kept in 0.4.0.

Doc-only (CHANGELOG.md, 13+/45-); build green, only CHANGELOG.md changed. Independently accuracy-reviewed (git-forensics re-run) — all corrections verified, no true claim destroyed. Honest residuals: per-release test counts cannot be reconstructed (no v0.4.0/v0.5.0 tags) and were left, not guessed.

Commit: 448ad61.
<!-- SECTION:FINAL_SUMMARY:END -->
