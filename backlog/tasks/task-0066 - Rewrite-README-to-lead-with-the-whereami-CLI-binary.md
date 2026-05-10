---
id: TASK-0066
title: Rewrite README to lead with the whereami CLI binary
status: To Do
assignee: []
created_date: '2026-05-10 10:58'
labels:
  - docs
  - readme
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
README's Quickstart tells users to use socat against the TCP socket. The real first-party CLI (whereami locate / scan / stats / debug / version / history) is never mentioned. New users without WiGLE creds also see a warn about WiGLE and conclude the daemon is broken; need to advertise Apple WPS as zero-config primary. Surface the Justfile (just build / test / lint / e2e / fuzz). Found in v0.4.0 review (Explore docs review).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Quickstart leads with cargo run --bin whereami -- locate (or installed binary)
- [ ] #2 history command is documented with at least one example invocation
- [ ] #3 Apple WPS zero-config is called out: 'works without any credentials; WiGLE is optional secondary'
- [ ] #4 Justfile recipes (just build, test, lint, e2e, fuzz, qa) are surfaced
- [ ] #5 Section ordering: pitch -> what -> why -> quickstart (with CLI) -> CLI usage -> protocol (link to docs/protocol.md) -> config -> deployment (NixOS + home-manager) -> architecture -> dev -> status -> license
<!-- AC:END -->
