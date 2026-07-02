---
id: TASK-0184
title: parse_range panics on chrono overflow (reachable from any TCP history request)
status: Done
assignee:
  - '@claude'
created_date: '2026-07-02 04:49'
updated_date: '2026-07-02 06:35'
labels:
  - correctness
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
parse_range parses the count as any positive i64 and feeds it into panicking chrono APIs: Duration::days(n) etc panic above TimeDelta bounds, and now - dur panics when the year underflows (~262144). {"cmd":"history","range":"100000000d"} (10 bytes, well inside the 64 KiB request cap) deterministically panics the connection task; the client gets a closed socket instead of the protocol-mandated error JSON. Also crashes whereami history --range typos. The uncommitted Kani harness parse_range_no_panic (32-byte domain) is a live counterexample - the harness is correct, production is not. Ref whereamid/src/history.rs:139-158, server.rs:919.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 parse_range returns Err (never panics) for any i64 magnitude and any unit; regression tests cover both overflow sites (TimeDelta construction and DateTime subtraction)
- [x] #2 a TCP history request with a huge range yields an ok:false error response and the connection survives
- [ ] #3 the Kani parse_range_no_panic harness passes against the fixed code
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. history.rs parse_range: swap panicking chrono constructors Duration::{seconds,minutes,hours,days,weeks}(n) for TimeDelta::try_*(n) (Option), map None -> anyhow Err (too large). Replace now - dur with now.checked_sub_signed(dur), map None -> Err (date under/overflow). Switch import Duration -> TimeDelta.
2. Regression tests (AC1): 99999999999999999s hits TimeDelta-construction site; 100000000d hits DateTime-subtraction site; plus i64::MAX per unit. Assert Err, never panic.
3. AC2: tokio::test in server.rs building a real DaemonState (in-memory DB) and calling dispatch_command(History{range:huge}); assert ok:false JSON + no panic (connection survives).
4. AC3: cannot run Kani here (broken host toolchain); reason in notes why parse_range_no_panic now holds (all constructors Option-guarded; subtraction checked).
5. Gate: nix develop -c just e2e + fmt. Commit, no push, no AI attribution.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented (commit 5d4a5b0).

- Root cause: parse_range used the panicking chrono constructors (Duration::days/seconds/... panic above TimeDelta bounds) and DateTime subtraction (now - dur panics on year under/overflow). Two distinct panic sites.
- Fix: TimeDelta::try_{seconds,minutes,hours,days,weeks}(n) (return Option) + now.checked_sub_signed(dur) (returns Option); both None mapped to the existing anyhow Err path. Import switched chrono::Duration -> TimeDelta (Duration was only used here).
- Verified magnitudes: 99999999999999999s trips TimeDelta construction (site 1); 100000000d builds a valid TimeDelta but overflows the year subtraction (site 2). NOTE: 10^7 weeks (~191k yrs) is STILL representable (DateTime floor ~ year -262144), so the weeks regression case uses 10^8 weeks (~1.9M yrs) to actually cross the floor.

Gate (nix develop -c just e2e + fmt): exit 0/0. Test totals 206 -> 208 (+parse_range_returns_err_on_overflow_never_panics in whereamid bin; +history_huge_range_yields_ok_false_not_panic dispatch test). clippy -D warnings clean, fmt clean.

AC3 (Kani) HONEST STATUS: Kani was NOT executed here - host toolchain is broken (per task setup). The property parse_range_no_panic now holds by construction: the only panic-capable operations in parse_range are (1) the UTF-8 split (already boundary-safe since task-0048), (2) i64 parse (returns Result), (3) the chrono constructors (now Option via try_*), (4) the subtraction (now Option via checked_sub_signed). spec.len()-unit_len cannot underflow because unit_len=last_char.len_utf8()<=spec.len(). So parse_range is total (Result, never panics) over every bounded UTF-8 input the harness enumerates. Executed equivalent: parse_range_returns_err_on_overflow_never_panics exercises the exact overflow inputs. Run out-of-band: cd whereamid && cargo kani --harness parse_range_no_panic.

Review-gate honesty correction (orchestrator): AC#3 (the Kani parse_range_no_panic harness passes) is UNCHECKED - Kani cannot be run in this environment (broken host rustup toolchain + NixOS linker; see task-0082). The behavioral equivalent is verified by parse_range_returns_err_on_overflow_never_panics and the tokio dispatch test (ok:false, no panic). The Kani re-verification remains pending under the out-of-band Kani setup (task-0082/0181). Code fix is complete and reviewed GO.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Fixed a reachable panic: parse_range fed the parsed count into panicking chrono APIs, so a 10-byte TCP history request ({"cmd":"history","range":"100000000d"}) deterministically panicked the connection task and dropped the client socket instead of returning the protocol error JSON.

Changes:
- history.rs: replaced Duration::{seconds,minutes,hours,days,weeks}(n) with the fallible TimeDelta::try_* constructors and now-dur with now.checked_sub_signed(dur); both overflow paths map to the existing anyhow Err. Import Duration -> TimeDelta.
- Tests: parse_range_returns_err_on_overflow_never_panics covers both overflow sites (TimeDelta construction via 99999999999999999s; DateTime subtraction via 100000000d) plus i64::MAX x every unit; server::history_dispatch_tests::history_huge_range_yields_ok_false_not_panic drives the full dispatch path against an in-memory DaemonState and asserts an ok:false envelope with no panic.

User impact: malformed/huge --range values (and typos) now return a clean error instead of crashing the connection task.

Gate: just e2e + fmt exit 0; 206 -> 208 tests.

Risk/follow-up: AC3 requires the Kani harness parse_range_no_panic to pass; Kani could NOT be executed here (broken host toolchain). Property argued by construction + covered by the equivalent executed unit test; needs an out-of-band cargo kani run to formally close.
<!-- SECTION:FINAL_SUMMARY:END -->
