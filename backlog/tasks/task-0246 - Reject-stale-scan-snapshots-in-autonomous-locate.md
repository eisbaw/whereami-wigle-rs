---
id: TASK-0246
title: Reject stale scan snapshots in autonomous locate
status: Done
assignee:
  - task0246-implementer
created_date: '2026-07-05 14:24'
updated_date: '2026-07-05 17:44'
labels:
  - correctness
  - history
dependencies:
  - TASK-0096
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0096 intentionally consumes the existing Debouncer rather than owning a second scanner. The Debouncer retains its last stable snapshot indefinitely across scanner failures, so an autonomous loop can otherwise append stationary fixes long after the last real observation. Define and enforce an explicit freshness policy without changing interactive locate semantics.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Define a documented monotonic maximum scan-age policy for autolocate, including its relationship to scan cadence and configurable intervals.
- [x] #2 Autolocate skips before cache touch, plausibility, last_fix mutation, history append, and counter increment when the latest scan is absent or stale; interactive locate behavior remains unchanged.
- [x] #3 Expose any new threshold through Rust CLI and shared NixOS/Home Manager options with zero/disable semantics pinned and flag/default parity tested.
- [x] #4 Add deterministic fresh-boundary, stale-boundary, repeated scanner-failure, recovery-after-new-scan, wipe, and shutdown tests.
- [x] #5 Document that freshness prevents autonomous history from treating an old Wi-Fi snapshot as new motion evidence.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add a Linux CLOCK_BOOTTIME newtype/reader and attach optional confirmed-refresh evidence atomically to each Debouncer sample while retaining existing received/wall timestamps for scan/debug output.
2. Make NetworkManager confirmation capture its unique D-Bus owner, resolve the Wi-Fi device path through that owner, and baseline the raw signed Device.Wireless.LastScan value through the same owner. Treat a negative baseline, including the unset -1 value, conservatively as unconfirmed because the first cycle cannot prove strict advancement. For a nonnegative baseline, require a strictly advanced, nonfuture LastScan generation at or after the CLOCK_BOOTTIME attempt start under the shared refresh deadline.
3. After generation proof, relist with nmcli --rescan no and confirm that output only after a same-owner, same-device-path, nonregressing/nonfuture LastScan fence plus a trailing unique-owner recheck. A successful nmcli command without the complete proof remains usable only as unconfirmed cached output.
4. Confirm only non-aborted stdout from a successful blocking iw dev <iface> scan. Retry numeric EBUSY failures; after a generic nonzero result, exhausted busy retries, or the successful-but-aborted sentinel, allow a separate iw scan dump only as unconfirmed interactive fallback. Process-start failure, timeout, ENETDOWN, or a failed dump publishes nothing.
5. Add --autolocate-max-scan-age-secs and the shared Nix option, default 180, range 0..=31536000. Zero is an explicit legacy override that disables age and confirmation rejection but never makes an absent snapshot present; autolocate interval zero remains the loop off-switch.
6. At autonomous locate admission, under privacy plus Debouncer ownership, classify absent/unconfirmed/stale/fresh before candidate construction or any DB/cache/plausibility/last_fix/history/counter work. Exact equality is fresh; max+1ns is stale. Interactive locate remains unchanged.
7. Add deterministic classifier, scanner-proof, boundary, repeated-error, recovery, wipe, shutdown, no-side-effect, interactive-regression, config/Nix parity/bounds, and suspend-inclusive clock tests. Update README, PRD, and ADR-010.
8. Run targeted tests and just e2e, then mandatory parallel qa-test-runner and mped-architect review. Fix all findings, re-run gates on a frozen tree, close via backlog CLI, and commit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Preflight contract pinned after independent QA/architecture audits. Freshness is based on confirmed radio-refresh evidence, not merely the time cached output was received. CLOCK_BOOTTIME is required so suspend time ages snapshots. The 180-second default is three times the default 60-second steady scan cadence and allows bounded scanner latency; it is independent of the 120-second autolocate cadence and is not dynamically widened. Operators using custom scan cadence should budget scan interval plus runtime/jitter. With a positive maximum, absent, unconfirmed, or older-than-maximum snapshots skip before all autonomous side effects. At exact maximum age the snapshot is fresh. Zero explicitly opts into legacy present-snapshot behavior, including unconfirmed/ancient data; absence still skips. Expected skips are debug-level outcomes; boot-clock failures/impossible future stamps fail closed with actionable warnings. Wipe clears sample and evidence atomically; recovery requires a new confirmed scan.

NetworkManager proof is deliberately stronger than a successful nmcli exit. The scanner captures the NetworkManager unique D-Bus owner, targets that owner for both GetDeviceByIpIface and raw Device.Wireless.LastScan reads, and conservatively rejects a negative baseline, including -1, because that first cycle cannot demonstrate strict generation advancement. A usable proof requires a nonnegative LastScan to advance strictly beyond baseline, be no earlier than the attempt-start ceiling, be no later than the observation clock, and arrive before the shared deadline. The scanner then performs a post-proof nmcli --rescan no relist and fences the retained output with the same unique owner/device path, a nonregressing nonfuture generation, and a trailing owner check. Missing or failed proof leaves nonempty nmcli output interactive-only and unconfirmed.

The iw path confirms only stdout from a successful blocking iw dev <iface> scan attempt. Numeric EBUSY failures are retried. A generic nonzero result, exhausted busy retries, or the successful-but-aborted sentinel may use a separate iw scan dump only as unconfirmed interactive fallback; process-start failure, timeout, and ENETDOWN remain total errors and publish no sample. A failed dump also publishes no sample. These distinctions prevent command success or readable cache state from being mistaken for new autonomous motion evidence. The same implementer remains the sole source writer.

Final gates on the frozen implementation: exact nix-shell --run just-e2e passed with 709 tests, documentation tests, and strict all-target Clippy; exact-source Nix flag-parity, option-bounds, package release, and full VM checks passed; cargo fmt and git diff hygiene passed; draw.io XML, GUI visibility, PNG export, and visual inspection passed. Mandatory parallel qa-test-runner and mped-architect reviews initially found stale tracker/PRD/ADR wording; the same implementer corrected it, fresh QA reran exact E2E, and both final reviewers returned GO with no findings. Architecture explicitly judged scanner.rs acceptably organized because production remains cohesive and the remaining bulk is the exhaustive private proof matrix.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Autonomous locate now accepts only present, confirmed, CLOCK_BOOTTIME-fresh scan snapshots before any cache, database, plausibility, last-fix, history, or counter side effect. NetworkManager confirmation is fenced by unique owner, device path, strictly advanced raw LastScan generation, post-proof relist, final generation checks, and a trailing owner check; blocking iw scans are confirmed only on genuine success, with cached dumps retained solely for interactive unconfirmed fallback. The new Rust and shared Nix max-age setting defaults to 180 seconds, treats exact equality as fresh, max plus 1ns as stale, and uses zero only as an explicit legacy present-snapshot override. Wipe, recovery, shutdown, parity, package, VM, documentation, and diagram coverage were added and all required gates passed.
<!-- SECTION:FINAL_SUMMARY:END -->
