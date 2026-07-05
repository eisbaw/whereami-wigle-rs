---
id: TASK-0090
title: 'whereami-client: add read timeout on daemon socket'
status: Done
assignee:
  - task0090-implementer
created_date: '2026-05-13 21:55'
updated_date: '2026-07-05 10:44'
labels:
  - robustness
  - client
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
WhereAmIClient's shared exchange path previously called read_line with no timeout, so a half-open TCP or Unix connection (network partition with no FIN, daemon killed mid-write, or internal stall) could block every typed and raw caller forever. The daemon's five-second WRITE_TIMEOUT does not bound command dispatch: one BSSID can spend 15 seconds in Apple and 15 seconds in WiGLE, while batches and custom daemon timeouts can take longer. Apply configurable read/write socket timeouts in the shared exchange. The documented default is 40 seconds: the nominal 15s + 15s provider chain and 5s write envelope, plus 5s scheduling/DB margin. Library and CLI users must be able to raise or explicitly disable it for legitimately long operations; it is an inactivity policy, not an end-to-end server deadline.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 WhereAmIClient::exchange configures a read timeout on TcpStream before reading
- [x] #2 raw_command and every typed accessor share exchange; UnixStream receives the same timeout policy
- [x] #3 the documented default is at least 2x daemon WRITE_TIMEOUT and honestly records the 40s rationale and limits
- [x] #4 one library default exists, with library and CLI override plus explicit disable
- [x] #5 a real TCP peer accepts but never writes; the client returns a timeout-shaped error within the configured bound
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add a documented default 40-second client I/O budget: 35-second nominal provider/write envelope plus 5-second scheduling/DB margin, while acknowledging arbitrary batches have no finite universal bound.
2. Store Option<Duration> on WhereAmIClient and expose builder override/disable plus a global CLI --timeout-secs option (0 disables).
3. Apply each client's read/write timeout to both TCP and Unix streams inside shared exchange; preserve actionable timeout errors and OS sources.
4. Use a deterministic silent real TCP peer with an injected short timeout and independent watchdog; separately pin the production default and CLI parsing/wiring.
5. Run focused tests, formatting, lint, fresh mandatory parallel reviews, and final E2E.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from task-0196: the client has two transports (UnixStream and opt-in TcpStream) unified through private Transport + WhereAmIClient::exchange; raw and typed commands share that path. Timeout configuration therefore belongs in exchange and must cover both transports.

Review history: the first frozen design used a universal 10s timeout. Mandatory architecture review rejected it because daemon WRITE_TIMEOUT begins only after dispatch; one uncached BSSID can spend 15s in Apple then 15s in WiGLE, and larger/configured batches have no finite universal bound. The redesign uses a documented 40s default (35s nominal provider/write envelope plus 5s margin), library and CLI override/disable, and explicitly states this is an operator inactivity policy rather than an end-to-end server guarantee. The second frozen code/API design passed functional review, but both gates rejected stale tracker wording that still said 10/35s; this edit corrects the durable record.

Final implementation: DEFAULT_SOCKET_IO_TIMEOUT is 40s. WhereAmIClient stores Option<Duration>, exposes validated with_io_timeout and explicit without_io_timeout builders, and applies the option to read/write timeouts on both TcpStream and UnixStream inside shared exchange. The CLI exposes global --timeout-secs; 0 disables. Timeout errors include the configured duration and retain the OS TimedOut/WouldBlock source. README and help explain long-operation overrides.

Validation: real TCP silent-peer typed locate test injected 250ms and returned in ~258ms; default/custom/disabled library and CLI states, Unix forwarding, thread/watchdog cleanup, 50 client tests, rustdoc -D warnings, strict clippy, fmt, and 502-test E2E all passed. Final frozen full hash fb4edf1eb3a11889402ad6c314ed4b4c127af7091b61835c8b89ffb4b1e44523 received QA GO and architecture GO after two earlier NO-GO rounds corrected the unsafe universal 10s premise and stale tracker wording.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Client socket reads/writes are now bounded by a documented 40-second default across typed/raw TCP/Unix paths. Library callers and CLI users can raise the inactivity timeout or explicitly disable it for supervised long operations. A deterministic real silent-peer test proves timeout error shape, timing, source preservation, and finite cleanup.
<!-- SECTION:FINAL_SUMMARY:END -->
