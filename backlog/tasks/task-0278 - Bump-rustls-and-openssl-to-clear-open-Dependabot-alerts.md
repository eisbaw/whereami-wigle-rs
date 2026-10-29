---
id: TASK-0278
title: Bump rustls and openssl to clear open Dependabot alerts
status: To Do
assignee: []
created_date: '2026-10-29T11:49:00Z'
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
- [ ] #1 Cargo.lock and whereamid/fuzz/Cargo.lock resolve rustls >= 0.23.46 and openssl >= 0.10.80
- [ ] #2 just e2e passes and nix build .#default succeeds on the bumped lockfiles
- [ ] #3 After push, GitHub reports no open Dependabot alerts for these advisories
<!-- AC:END -->
