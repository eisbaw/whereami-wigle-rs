# ADR-001: TCP JSON-lines, not HTTP

**Status**: Accepted — **amended by [ADR-003](003-unix-socket-access-control.md)**:
the default transport is now a Unix domain socket (access-controlled by
filesystem permissions); TCP is opt-in. The JSON-lines framing decided here
still stands and is unchanged on both transports.
**Date**: 2026-05-03

## Context

The daemon needs an IPC mechanism for clients to query location. Options: HTTP/REST, gRPC, Unix socket, raw TCP.

## Decision

Raw TCP on localhost with newline-delimited JSON. One-shot connections: connect, send one JSON line, receive one JSON line, close.

## Rationale

- Any language with a socket and JSON parser can be a client. No HTTP framework, no protobuf codegen.
- One-shot avoids request multiplexing, connection state, request IDs. The local handshake is sub-millisecond on both a unix socket and loopback TCP.
- No TLS needed for local IPC. **Note (ADR-003):** the original "bound to 127.0.0.1, so it is safe" reasoning addressed *network* exposure only and missed the *multi-user* threat — on a shared host every local UID could read location history and drive WiGLE quota over loopback TCP. Access control is now the unix socket's filesystem permissions (ADR-003); TLS is still unnecessary.
- HTTP adds overhead (headers, chunked encoding, content negotiation) with zero benefit for local IPC.

## Consequences

- No browser-based clients without a proxy (non-goal anyway).
- No streaming/push — client must poll. Acceptable since location doesn't change second-by-second.
- Protocol versioning done via a `v` field in responses rather than URL paths.
