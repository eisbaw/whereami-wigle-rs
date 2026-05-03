# ADR-011: Scan backoff — fast then slow

**Status**: Accepted
**Date**: 2026-05-03

## Context

The daemon scans continuously in the background to keep the debounce ring buffer filled. Scanning has a cost: radio time, CPU, and nmcli spawns.

## Decision

Two phases: fast (default 10s) for the first N seconds after start, then slow (default 60s) steady state. Configurable via CLI args only.

## Rationale

- Fast phase fills the debounce buffer quickly (~50s to reach threshold of 5 with 10s intervals). First `locate` works ASAP after daemon start.
- Slow phase reduces load once the buffer is warm. AP environments don't change second-by-second.
- CLI args (not config file) because these are operational tuning, not secrets. Easy to override in systemd ExecStart.

## Consequences

- Cold-start delay of ~50s before debounce can classify any AP as stable (with defaults). Acceptable — on restart with warm cache, locate still works immediately for previously-seen APs.
- Fast phase burns more nmcli calls. On a laptop this is negligible.
