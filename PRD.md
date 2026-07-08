# whereami — PRD

## What whereami is

whereami is an offline-first Wi-Fi geolocation service for Linux. It scans the
local access points around you, resolves their BSSIDs to coordinates, caches
those results on your own disk, and answers "where am I?" without phoning home
once the surrounding area is cached. It exists because browser-based
geolocation ships your Wi-Fi environment to third-party servers on every query,
and the usable offline options on Linux have withered (GeoClue's Mozilla
backend is dead).

It ships as a long-running daemon (`whereamid`) plus a thin client
(`whereami`). The daemon owns a local SQLite cache and listens on an
access-controlled Unix domain socket (opt-in, unauthenticated TCP is also
available), speaking a simple newline-delimited JSON protocol — one JSON
request per line, one JSON response per line. On a cache miss it calls remote
location providers (Apple WPS as the primary, free, no-auth backend; WiGLE as
an optional secondary) and stores what it learns, so subsequent queries in the
same area are served locally.

The client is a typed Rust library over that socket, but the protocol is plain
JSON lines, so any language (or `curl`) can drive the daemon directly. Beyond
point-in-time location, the daemon keeps a last-known fix and a fix timeseries
for local location-history queries, under configurable retention windows.

## Authoritative sources of truth

This PRD is intentionally minimal and version-agnostic: it states what the
project is and why, and nothing that carries a version number or a field name.
The detailed, dated, and drift-prone material lives where it can stay honest
against the code. Prior to this retirement, a detailed PRD tried to enumerate
every CLI flag and wire field and repeatedly went stale; that content is
preserved at `cruft/PRD-v0.x-detailed.md` for history but is no longer
maintained.

For anything concrete, consult these instead — they are kept current with the
code:

- **`README.md`** — user-facing overview, install/build/run, and CLI usage.
- **`docs/protocol.md`** — the wire protocol and response shapes; the live
  source of truth for every request command and response field.
- **`CHANGELOG.md`** — version history and per-release changes.
- **`backlog/`** (via the `backlog` CLI) — current work in flight and the
  design decisions behind it.

If you need the exact CLI flags or the exact JSON response fields, read
`README.md` and `docs/protocol.md` rather than this file. Duplicating them here
is precisely the drift this retirement removes.
