---
id: TASK-0278
title: Bump rustls and openssl to clear open Dependabot alerts
status: In Progress
assignee:
  - '@claude'
created_date: '2026-10-29T11:49:00Z'
updated_date: '2026-10-29T12:04:00Z'
labels:
  - deps
  - security
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
GitHub Dependabot reports open alerts against Cargo.lock: rustls < 0.23.46 (GHSA-w44r-93j7-qv28, GHSA-c8m5-c952-6cm4, GHSA-h2v8-394m-p7vv, GHSA-2mjx-qc3c-rqvc) and openssl < 0.10.80 (GHSA-phqj-4mhp-q6mq). Dependabot PR #2 (openssl only) has been open since 2026-05-21. Lockfile-level patch bumps should clear all of them.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 just e2e passes and nix build .#default succeeds on the bumped lockfiles
- [ ] #2 After push, GitHub reports no open Dependabot alerts for these advisories
- [x] #3 Cargo.lock resolves rustls >= 0.23.46 and openssl >= 0.10.80 (whereamid/fuzz/Cargo.lock is gitignored and regenerated locally)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. cargo update -p rustls -p openssl -p openssl-sys in / and whereamid/fuzz (lockfile-only, semver-compatible)
2. Verify resolved versions >= patched
3. just e2e + nix build .#default
4. Commit; ask before push
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
- cargo update -p rustls -p openssl -p openssl-sys: rustls 0.23.40->0.23.46, rustls-webpki 0.103.13->0.103.16, openssl 0.10.79->0.10.81, openssl-sys 0.9.115->0.9.117. No other lockfile changes.
- whereamid/fuzz/Cargo.lock is gitignored (Dependabot PR #2 predates that); regenerated locally, also resolves the patched versions.
- Gates: just e2e exit 0; nix build .#default exit 0. Done in a worktree to keep the uncommitted task-0121/0272 work out of the gate.
- AC#2 pending a push (needs approval).
<!-- SECTION:NOTES:END -->
