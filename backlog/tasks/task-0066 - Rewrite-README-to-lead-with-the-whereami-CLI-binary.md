---
id: TASK-0066
title: Rewrite README to lead with the whereami CLI binary
status: Done
assignee: []
created_date: '2026-05-10 10:58'
updated_date: '2026-05-10 14:28'
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
- [x] #1 Quickstart leads with cargo run --bin whereami -- locate (or installed binary)
- [x] #2 history command is documented with at least one example invocation
- [x] #3 Apple WPS zero-config is called out: 'works without any credentials; WiGLE is optional secondary'
- [x] #4 Justfile recipes (just build, test, lint, e2e, fuzz, qa) are surfaced
- [x] #5 Section ordering: pitch -> what -> why -> quickstart (with CLI) -> CLI usage -> protocol (link to docs/protocol.md) -> config -> deployment (NixOS + home-manager) -> architecture -> dev -> status -> license
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Rewrote README to lead with the whereami CLI binary (cargo run --bin whereami -- locate). socat example removed. New structure: pitch (with sample output) -> Why -> Quickstart -> CLI usage -> Configuration -> How it works -> Accuracy -> Caching/offline -> Development (Justfile) -> NixOS module (mentions homeManagerModules) -> Protocol (links to docs/protocol.md) -> Architecture -> License. Apple WPS zero-config explicitly called out twice.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
README rewritten. Leads with the CLI binary, advertises Apple WPS zero-config, lists all 16 CLI flags, surfaces Justfile recipes, links to docs/protocol.md and CHANGELOG.md, mentions home-manager module.
<!-- SECTION:FINAL_SUMMARY:END -->
