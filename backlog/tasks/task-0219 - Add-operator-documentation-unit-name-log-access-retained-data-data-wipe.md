---
id: TASK-0219
title: 'Add operator documentation: unit name, log access, retained data, data-wipe'
status: To Do
assignee: []
created_date: '2026-07-02 04:54'
updated_date: '2026-07-03 09:10'
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

Forward-carry from task-0204/0205/0206 batch (commits f6bb2da, effa4a8, b6aa6f2). New operator-facing surface to document:
- Secrets (task-0205): the system service reads WiGLE creds from services.whereami.credentialsFile via systemd LoadCredential. Operators SHOULD use a 0600 root:root file (e.g. /etc/whereami/secrets.toml) — it is delivered to the DynamicUser through the credentials tmpfs and its mode is never widened. Startup logs "WiGLE credentials configured" (or "... not configured"). Do NOT put secrets in a world-readable location.
- Retention (task-0206): historyRetentionDays is now a module option (default 30; 0 disables pruning). Previously frozen at the compiled default — the retained-personal-data section should reference the option, not just the flag. Five more tunables were also exposed (address-cache-ttl-days, http/nominatim timeouts, history segment distance/min-duration).
- Restart (task-0204): the unit is Restart=on-failure with a bounded start limit (5/60s); document that a persistently mis-configured unit lands in `failed` (systemctl status whereamid) rather than hot-looping.
<!-- SECTION:NOTES:END -->
