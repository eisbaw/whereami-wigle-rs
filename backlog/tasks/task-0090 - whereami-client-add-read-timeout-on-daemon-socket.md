---
id: TASK-0090
title: 'whereami-client: add read timeout on daemon socket'
status: To Do
assignee: []
created_date: '2026-05-13 21:55'
updated_date: '2026-07-03 05:50'
labels:
  - robustness
  - client
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
WhereAmIClient::request calls reader.read_line(&mut response_line) with no timeout. The daemon side has WRITE_TIMEOUT=5s (task-0080) so a deadlocked daemon will normally close the socket and the client sees EOF. But a half-open connection (network partition with no FIN, NAT timeout, killed-9 daemon mid-write) leaves the client blocked forever on read. Symptom: interactive 'whereami locate' from a shell script hangs and wedges any caller / wrapper. Fix: stream.set_read_timeout(Some(Duration::from_secs(N))) before constructing the BufReader. Same for raw_command. Default N=10s is generous given the daemon's WRITE_TIMEOUT=5s.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 WhereAmIClient::request sets a read timeout on the TcpStream before reading
- [ ] #2 raw_command does the same
- [ ] #3 Read timeout is at least 2x the daemon's WRITE_TIMEOUT to avoid spurious failures
- [ ] #4 Timeout value is hardcoded (single source of truth in lib.rs) or exposed as a builder option; pick the simpler
- [ ] #5 Test: connect to a port that accepts but never writes; assert request() errors with a timeout-shaped error within the bound
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
forward-carried from task-0196: the client now has TWO transports — unix (std::os::unix::net::UnixStream) and opt-in TCP (std::net::TcpStream) — unified via the private Transport trait + WhereAmIClient::exchange() in whereami-client/src/lib.rs. There is still NO read/write timeout on either branch (raw_command -> exchange). Add the timeout inside exchange() so it covers BOTH branches at once (set_read_timeout/set_write_timeout exist on both UnixStream and TcpStream).
<!-- SECTION:NOTES:END -->
