# ADR-010: nmcli as primary scanner, iw as fallback

**Status**: Accepted
**Date**: 2026-05-03

## Context

Wi-Fi scanning on Linux requires either CAP_NET_ADMIN (for `iw scan trigger`) or cooperation from a privileged service. Running the daemon as root is undesirable.

## Decision

Use `nmcli device wifi rescan` + `nmcli device wifi list` as the primary scan method. Fall back to `iw dev <iface> scan trigger/dump` only when nmcli is unavailable.

## Rationale

- NetworkManager already runs as root and handles scan triggers. nmcli talks to it over D-Bus — no special privileges needed by the caller.
- `iw scan trigger` fails with "Operation not permitted" without CAP_NET_ADMIN. `iw scan dump` returns stale kernel cache that goes cold quickly.
- nmcli returns fresh results on every call because it triggers a rescan through NetworkManager.
- Most Linux desktops run NetworkManager. Headless/minimal systems that don't can use the iw fallback with setcap.

## Consequences

- Dependency on NetworkManager for the common case. Systems using iwd or wpa_supplicant directly need the iw path + capabilities.
- nmcli terse output (`-t`) escapes colons in BSSIDs with backslashes. Parser must handle this.
- Signal is reported as 0-100% by nmcli, converted to approximate dBm (-90 + pct*60/100). Less precise than iw's actual dBm values.
