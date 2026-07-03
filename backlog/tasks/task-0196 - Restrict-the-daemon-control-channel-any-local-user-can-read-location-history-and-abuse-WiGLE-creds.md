---
id: TASK-0196
title: >-
  Restrict the daemon control channel: any local user can read location history
  and abuse WiGLE creds
status: Done
assignee:
  - '@mped-architect'
created_date: '2026-07-02 04:52'
updated_date: '2026-07-03 06:14'
labels:
  - security
  - privacy
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
run_server binds TCP and dispatches every accepted line with zero peer identification (no SO_PEERCRED/auth). Any local process, any UID, can nc 127.0.0.1:4747 and get history (full stay-point history), locate (live position), scan (surrounding BSSIDs/SSIDs), stats, and resolve - which makes the daemon issue WiGLE calls under the owner's credentials (burn quota + proxy arbitrary lookups). ADR-001 only argued no-TLS-because-localhost; the multi-user threat is unexamined. Aggravator: --bind is a raw String with no loopback guard, so --bind 0.0.0.0:4747 silently exposes all of this to the LAN. Ref whereamid/src/server.rs:312-395, config.rs:12-14, docs/adr/001-tcp-json-lines.md.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 a process under a different UID than the daemon cannot obtain locate/history/scan/stats/resolve results (Unix socket + SO_PEERCRED, or a token from the 0600 config)
- [x] #2 the owner's own client still works with no manual credential step on a default install
- [x] #3 binding to a non-loopback address requires an explicit opt-in and emits a warning naming the exposure
- [x] #4 protocol.md documents the access-control model and the multi-user threat
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Access control = Unix domain socket gated by filesystem permissions (Unix-native model). No token scheme.

DAEMON (whereamid):
1. config.rs: --bind becomes Option<String> (no default); add --socket Option<String> (no clap default; computed at startup); add --allow-nonlocal-bind bool. validate(): parse --bind as SocketAddr (fail on garbage); non-loopback requires --allow-nonlocal-bind (error names flag); loopback ok without flag. Pure parsed_bind() helper as single source. Unit tests alongside existing.
2. server.rs: add self_bind_unix(path) [stale-socket probe: connect-> live=bail, refused=unlink+bind; then chmod 0600 + fail-closed re-stat mirroring db.rs 53aba50; bail if perms did not take]. Add sd_listen_fd() (~15-line LISTEN_PID/LISTEN_FDS parse; >1 = error). run_server(state, unix_listener): binds optional TCP from state.args.bind INSIDE run_server (preserves task-0195 self-exit + existing exit-0-on-bind-fail test); single tokio::select! accept loop over unix + optional-TCP (pending() when None); handle_connection generic over S: AsyncRead+AsyncWrite+Unpin+Send via tokio::io::split; debug-log SO_PEERCRED on unix accepts (log-only, no in-proc UID enforcement). Warn on loopback TCP; loud warn on non-loopback.
3. main.rs: compute effective socket path (--socket else $XDG_RUNTIME_DIR/whereamid.sock else /run/whereami/whereamid.sock); socket-activated fd wins (skip unlink/chmod/cleanup). Bind unix in startup (fatal on live-socket -> nonzero exit). Pass listener to run_server. Best-effort unlink self-bound path after drain.

CLIENT (whereami-client):
4. lib.rs: ConnectTarget { Unix(PathBuf) | Tcp(String) }. Pure resolve_target(env values + exists predicate): WHEREAMI_SOCKET -> WHEREAMI_ADDR -> first existing of XDG/run defaults -> error listing probed paths + env hints. NO 127.0.0.1:4747 fallback (port-squat impersonation). from_env()->Result; new(target)/tcp()/unix() ctors. raw_command gains unix branch via a small Transport trait (shutdown_write) so the exchange dance stays single-source. Keep *_raw() SoT + no-read-timeout.
5. main.rs: route from_env() error through json/human split so --json still emits synthesized JSON (task-0183). Replace default_addr(). Update help text.

TASK-0231: sigpipe test sets WHEREAMI_SOCKET=<absent temp path> + env_remove WHEREAMI_ADDR on the child (tempfile is NOT a dep -> hand-roll path, shutdown.rs style). Grep tests for 4747.

NIX: options.nix add socketPathDefault param + bind(nullOr,null)/allowNonlocalBind/socketPath/allowedUsers. module.nix: users.groups.whereami.members; systemd.sockets.whereamid (ListenStream=socketPath, 0660 root:whereami, wantedBy sockets.target); service requires/after socket, keeps multi-user.target, no --socket. hm-module.nix: --socket %t/whereamid.sock self-bind. exec-start.nix: conditional --bind/--socket/--allow-nonlocal-bind.

DOCS: protocol.md Transport rewrite; new ADR-003; amend ADR-001; ADR README index; CHANGELOG; sweep README/PRD/client-README/ADR-002 for stale 4747.

TESTS: config validate cases; unix round-trip + mode 0600; stale recovery; live-socket refusal (binary, nonzero); client resolution order (pure); adjust shutdown.rs to pass explicit --socket.

GATE: nix develop -c just e2e + fmt. Commit per unit (daemon/client/nix/docs), backlog separate.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Onboarding done. GOTCHA: tempfile is NOT a direct dev-dep of either crate (orchestrator said it was); only transitive in Cargo.lock. Following shutdown.rs pattern (hand-rolled temp path, no tempfile dep) for 0231 to avoid adding a dependency.
Design decision: unix bind is fatal (startup, nonzero exit on live-socket) but TCP bind stays non-fatal (inside run_server, exit-0) to preserve task-0195 self-exit architecture + existing shutdown.rs bind-collision test. Asymmetry documented in ADR-003; may file follow-up to make explicit --bind failures fatal too.

IMPLEMENTATION COMPLETE. 4 code commits: 8f7da4f (daemon), f9bae28 (client), 8396cd1 (nix), f907340 (docs).

GATE (nix develop -c just e2e + just fmt), final run: client-lib 15, client-bin 22, sigpipe 1, daemon-lib 86, daemon-bin(main.rs, includes server+config+main+the double-counted lib modules) 167, proptests 11, shutdown 3. All pass. clippy --all-targets -D warnings: clean (exit 0). cargo fmt --check: clean (exit 0).

AC#1 DISCLOSURE (honest gap): cross-UID denial cannot be literally proven by single-UID cargo tests (a unit test cannot become another UID). The tick rests on the MECHANISM (unix socket + filesystem permissions: 0600 self-bind, 0660 root:whereami system+group) plus mode-bit assertions (self_bind_creates_0600_socket, unix_round_trip asserts 0600) and the live-socket refusal test. The actual cross-UID EACCES/allowed matrix is forward-carried to task-0209 VM test items (b)/(c). This mirrors the accepted task-0197 AC#1 precedent.

DESIGN ASYMMETRY (deliberate, filed as task-0233): unix bind is fatal at startup (nonzero exit on a live socket), but an opt-in TCP --bind failure stays non-fatal (swallowed in run_server -> task-0195 self-exit -> exit 0), to preserve the 0195 architecture and the existing shutdown.rs bind-collision test.

GOTCHAS: (1) tempfile is NOT a direct dev-dep (only transitive) despite the brief saying so — used the shutdown.rs hand-rolled-temp-path pattern instead. (2) the in-process unix round-trip test needs a multi_thread runtime because it drives a std BLOCKING client on a spawn_blocking thread (faithful to the real client). (3) module.nix relies on systemd auto-creating the /run/whereami parent dir on AF_UNIX bind — flagged for the 0209 VM test to verify.

Follow-ups filed: task-0232 (first-class client --socket/--addr flags; today env-var only), task-0233 (explicit --bind failure exit code). Forward-carried notes appended to 0209, 0090, 0206, 0216.

NIX honesty: modules are code-reviewed and nix-eval-verified (exec-start both variants; system socket unit + ExecStart + group + service wiring) but NOT machine-verified via flake checks / VM (task-0209).

Review gate (cycle 8): qa-test-runner GO + mped-architect GO, both on range 6083ae7..f907340. QA: two full gate runs byte-identical, 305/305 pass, clippy+fmt clean, hermeticity audit clean (no test can reach a live daemon; counterfactual construction attempted and failed), security tests verified non-tautological (0600 assertion proven load-bearing under umask 0022; live-socket refusal traced to nonzero exit through three independent guards).

In-thread hardening commit 946e266 addressed all actionable findings:
- M2: the earlier note here claimed the TCP-bind-failure exit-0 asymmetry was documented in ADR-003 — it was NOT at that time. ADR-003 Consequences now actually documents it (with the task-0233 ref). Doc-honesty defect class, caught by review.
- M3: socket-activated fd 3 is now VERIFIED to be a listening AF_UNIX socket (getsockname + SO_ACCEPTCONN, libsystemd sd_is_socket_unix parity) instead of trusted from LISTEN_* env; the unsafe comment no longer overclaims. Unit tests: listening socket passes, connected socketpair rejected, regular file rejected.
- L1: FD_CLOEXEC set on the adopted fd (systemd passes it without CLOEXEC; nmcli/iw children no longer inherit the control socket). Test proves the flag is SET, not defaulted.
- L2: self-bind now happens under umask 0077 (saved/restored) so the socket is never observable looser than 0600 even pre-chmod; chmod+re-stat kept as belt-and-braces.
- L3/L4: validate() rejects --allow-nonlocal-bind without --bind and empty/whitespace --socket.
- QA LOW-3: added tcp_round_trip_version_via_handle_connection (generic handler answers over a real TcpStream, ephemeral port).
Gate after hardening: 174 whereamid-bin tests (up from 167), all 9 binaries green x2 runs, clippy -D warnings clean, fmt clean.

Accepted/not-chased: QA LOW-4 (non-loopback bind covered at validate()+wiring level, no spawned-daemon e2e), QA LOW-5 (sigterm 300ms sleep pre-existing theoretical flake, stable across runs), QA MEDIUM-1 (real-systemd fd adoption untestable locally, 0209 VM scope; the new fd verification narrows the untested surface to tokio from_std registration).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Restricted the daemon control channel to a Unix domain socket gated by filesystem permissions (the Unix-native access-control model, ADR-003), closing the multi-user hole where any local UID could nc 127.0.0.1:4747 and read location history / live position / Wi-Fi scans and drive WiGLE quota under the owner's credentials.

Daemon:
- --bind is now opt-in (Option, default off) and PARSED as a SocketAddr (junk rejected at startup, not silently accepted); a non-loopback bind requires --allow-nonlocal-bind. New --socket flag.
- Serves on a unix socket by default ($XDG_RUNTIME_DIR/whereamid.sock else /run/whereami/whereamid.sock), chmod 0600 on self-bind with a fail-closed re-stat; stale sockets are recovered, live sockets never stolen. systemd socket activation (fd 3) is supported so a DynamicUser system service is reachable by the whereami group.
- One select! accept loop over unix + optional TCP; handle_connection is generic over the stream type; SO_PEERCRED is debug-logged, not enforced (the group model needs cross-UID connects).

Client:
- ConnectTarget { Unix | Tcp }; resolves WHEREAMI_SOCKET -> WHEREAMI_ADDR -> default socket paths, with NO implicit 127.0.0.1:4747 fallback (port-squat impersonation). --json still emits a synthesized JSON line on resolution failure (task-0183 parity).

Nix: bind becomes nullOr (default null); new socketPath/allowNonlocalBind/allowedUsers; system module adds a whereamid.socket unit (0660 root:whereami) + the whereami group; hm module self-binds under %t.

Docs: new ADR-003; ADR-001 amended; protocol.md Transport rewritten; CHANGELOG breaking-change entry; README/PRD/client-README/ADR-002 swept for stale 4747/TCP claims.

Tests: config validation; unix round-trip + mode 0600; stale-socket recovery; live-socket refusal (binary, nonzero exit); pure client resolution-order tests; every daemon-spawning test now passes an explicit --socket (hermetic).

Gate: e2e green (per-binary counts in notes), clippy --all-targets -D warnings clean, cargo fmt --check clean.

Risks/limits: AC#1 cross-UID denial is justified by the fs-perm mechanism + mode-bit assertions, NOT a literal cross-UID cargo test (forward-carried to the task-0209 VM test); opt-in TCP --bind failure still exits 0 (filed task-0233); nix is code-reviewed + nix-eval-verified but not VM-tested (task-0209).
<!-- SECTION:FINAL_SUMMARY:END -->
