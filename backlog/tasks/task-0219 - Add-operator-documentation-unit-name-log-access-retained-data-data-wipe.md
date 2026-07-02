---
id: TASK-0219
title: 'Add operator documentation: unit name, log access, retained data, data-wipe'
status: To Do
assignee: []
created_date: '2026-07-02 04:54'
updated_date: '2026-07-02 23:06'
labels:
  - docs
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
README documents enabling the service but never states the unit name whereamid, so journalctl -u whereamid is guesswork; RUST_LOG is mentioned only for the dev shell with no way to set it on the service. For a daemon whose product is a location-history database there is no doc of what is retained (fixes 30 days default; aps cache and last_fix indefinitely; path /var/lib/whereami/aps.sqlite) nor how to wipe it. No man page or shell completions exist. Ref README.md:174-190, nix/module.nix:15, whereamid/src/config.rs:87-101. Complements task-0202 (wipe facility) with the docs/ops side.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 docs state the systemd unit name and a one-line journalctl command, and how to set RUST_LOG via the module
- [ ] #2 docs enumerate retained personal data (tables, default retention, path) and a verified wipe procedure
- [ ] #3 man page/completions either shipped (clap_mangen/clap_complete) or explicitly declared out of scope
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from task-0197: retained data at rest is now owner-only. Document that aps.sqlite (fixes location-history, last_fix, all BSSID/SSID seen) is created mode 0600 and its state directory 0700, on system (/var/lib/whereami), hm (%h/.local/share/whereami) and manual/--db paths. The daemon also tightens an existing loose DB to 0600 on startup and warns if a pre-existing state dir is group/other-accessible. Relevant for the retained-data and data-access sections of the operator docs.
<!-- SECTION:NOTES:END -->
