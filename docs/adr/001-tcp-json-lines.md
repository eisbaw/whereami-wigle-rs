# ADR-001: TCP JSON-lines, not HTTP

**Status**: Accepted
**Date**: 2026-05-03

## Context

The daemon needs an IPC mechanism for clients to query location. Options: HTTP/REST, gRPC, Unix socket, raw TCP.

## Decision

Raw TCP on localhost with newline-delimited JSON. One-shot connections: connect, send one JSON line, receive one JSON line, close.

## Rationale

- Any language with a socket and JSON parser can be a client. No HTTP framework, no protobuf codegen.
- One-shot avoids request multiplexing, connection state, request IDs. Localhost TCP handshake is sub-millisecond.
- No TLS needed — bound to 127.0.0.1 only.
- HTTP adds overhead (headers, chunked encoding, content negotiation) with zero benefit for local IPC.

## Consequences

- No browser-based clients without a proxy (non-goal anyway).
- No streaming/push — client must poll. Acceptable since location doesn't change second-by-second.
- Protocol versioning done via a `v` field in responses rather than URL paths.
