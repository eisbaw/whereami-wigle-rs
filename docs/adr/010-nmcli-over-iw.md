# ADR-010: nmcli as primary scanner, iw as fallback

**Status**: Accepted
**Date**: 2026-05-03

## Context

Wi-Fi scanning on Linux requires either CAP_NET_ADMIN (for `iw scan`) or
cooperation from a privileged service. Running the daemon as root is
undesirable.

## Decision

Use `nmcli -t -f BSSID,SSID,SIGNAL,CHAN,FREQ device wifi list ifname <iface>
--rescan yes` as the primary refresh command, but do not trust its exit status
alone. NetworkManager can force-finish that command with cached rows and exit
zero after an internal request error.

Before the command, query the system bus with `busctl` for NetworkManager's
unique owner, the interface's device object path, and its signed-millisecond
`Device.Wireless.LastScan` generation. Address every device and property call
to that captured unique owner. Capture exact Linux `CLOCK_BOOTTIME` and
accept a polled generation only when the valid baseline was nonnegative, the
generation strictly advanced, and it is at least `ceil(start_time_ms)` but not
in the future. A `-1` baseline makes the cycle unconfirmed because
NetworkManager can update the first generation at scan start as well as scan
completion.

After proof, re-list with `--rescan no` so retained rows cannot predate the
generation. Re-resolve the unique owner and device path, then require the same
owner/path and a nonregressing, nonfuture final generation. Recheck the owner
after the final property read to close the restart race. Only that post-proof
list, including empty output, is confirmed. Any command, parse, timeout, owner,
path, clock, or generation failure downgrades readable output to unconfirmed. A
non-empty unconfirmed list remains available interactively; an empty or
unavailable one may proceed to `iw`.

The fallback uses blocking `iw dev <iface> scan`, not asynchronous `scan
trigger` plus a timed sleep and unrelated `scan dump`. Its successful output,
including empty output, is confirmed. Numeric `-EBUSY` responses are retried
independently of libc error-message wording; each retry owns a new timestamp
and only the successful retry may supply evidence. Generic
failure or exhausted busy retries may read a separate cached `scan dump`, which
is always unconfirmed. The zero-exit `scan aborted!` sentinel is also routed to
the unconfirmed dump rather than treated as a confirmed empty scan. Spawn
failure, timeout, and `Network is down` remain total errors and publish nothing.

Each evidence timestamp is the conservative start of the command whose later
generation was proved, using Linux `CLOCK_BOOTTIME`, so suspend ages the
snapshot. Cached commands have no evidence path and cannot borrow a timestamp
from a failed or unverifiable forced attempt. Forced nmcli plus generation
polling share one deadline; every child uses kill-on-drop. A total scanner error
leaves the prior snapshot aging.

## Rationale

- NetworkManager already runs as root and handles scan triggers. nmcli talks to it over D-Bus — no special privileges needed by the caller.
- Blocking `iw scan` fails with "Operation not permitted" without CAP_NET_ADMIN.
  `iw scan dump` returns stale kernel cache that goes cold quickly.
- The combined forced nmcli list removes the old asynchronous acknowledgement
  gap, while `LastScan` advancement and the post-proof relist close nmcli's
  zero-exit cached-output path. Unique-owner fencing prevents object-path reuse
  across a NetworkManager restart from masquerading as one generation.
- Most Linux desktops run NetworkManager. Headless/minimal systems that don't can use the iw fallback with setcap.

## Consequences

- Dependency on NetworkManager plus systemd's `busctl` for the common case.
  Systems using iwd or wpa_supplicant directly need the iw path + capabilities.
- nmcli terse output (`-t`) escapes colons in BSSIDs with backslashes. Parser must handle this.
- Signal is reported as 0-100% by nmcli, converted to approximate dBm (-90 + pct*60/100). Less precise than iw's actual dBm values.
- Interactive locate/scan/debug retain cached-output continuity. Autonomous
  history defaults to a 180-second confirmed-refresh maximum, preventing a
  retained Wi-Fi snapshot from being recorded as new motion evidence after
  scanner failure or suspend. Operators can explicitly restore legacy
  present-snapshot behavior with a zero freshness threshold.
