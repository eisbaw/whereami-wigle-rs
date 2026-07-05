# `wifiInterfaceExtraDoc` lets a caller append module-specific caveats to the
# wifiInterface description. Default "" keeps the system module's docs clean; the
# home-manager module passes the NetworkManager-only scanning caveat (task-0208).
{ lib, dbPathDefault, credentialsFileDefault, socketPathDefault, wifiInterfaceExtraDoc ? "" }:

# Numeric option types MIRROR whereamid's Args::validate() (config.rs) so a bad
# value fails `nixos-rebuild` evaluation instead of crash-looping the daemon at
# boot (task-0207). Policy:
#   - lib.types.ints.positive  (>= 1): the daemon bails when the value is 0/<=0.
#   - lib.types.ints.unsigned  (>= 0): the daemon accepts 0 — either because
#     validate() has NO check for it (scanFastDuration, dailyLimit) or because 0
#     is the documented "disabled" value (addressCacheTtlDays and retention
#     windows).
# Upper bounds are intentionally omitted for the other tuning options, so
# clap's numeric range remains their runtime guard. `maxConnections` and both
# retention windows have shared explicit Rust/Nix policy ranges.
let
  # Must match whereamid::db::MAX_RETENTION_DAYS.
  maxRetentionDays = 365000;
in {
  enable = lib.mkEnableOption "whereami Wi-Fi geolocation daemon";

  package = lib.mkOption {
    type = lib.types.package;
    description = ''
      The whereami package to use. Overrides must provide both bin/whereamid
      for the service and bin/whereami for operator commands; both modules
      install this package into their corresponding environment.
    '';
  };

  rustLog = lib.mkOption {
    type = lib.types.nullOr lib.types.str;
    default = null;
    example = "whereamid=debug";
    description = ''
      Optional RUST_LOG filter for the daemon service. null adds no
      unit-specific RUST_LOG assignment; when the service manager supplies no
      inherited value either, whereamid uses its built-in info fallback.
      Normal warning-level records can contain sensitive BSSIDs. Debug records
      can additionally contain resolved coordinates and Unix peer UID/GID/PID.
      Journal entries are outside the wipe facility, so enable debug only
      temporarily and manage journal retention separately. The filter must be
      a single line.
    '';
  };

  bind = lib.mkOption {
    type = lib.types.nullOr lib.types.str;
    default = null;
    description = ''
      TCP bind address (host:port) for whereamid. OPT-IN and UNAUTHENTICATED:
      when null (the default) the daemon serves ONLY on the unix control socket,
      whose access is gated by filesystem permissions. Any TCP listener exposes
      location history, live position, Wi-Fi scans and WiGLE quota with no
      authentication; a non-loopback address additionally requires
      allowNonlocalBind. See ADR-003.
    '';
  };

  allowNonlocalBind = lib.mkOption {
    type = lib.types.bool;
    default = false;
    description = ''
      Permit binding `bind` to a non-loopback address. Without it the daemon
      refuses to start on a non-loopback bind, because that exposes the
      unauthenticated TCP transport to other hosts. See ADR-003.
    '';
  };

  socketPath = lib.mkOption {
    type = lib.types.str;
    default = socketPathDefault;
    description = ''
      Path to the unix domain control socket (the canonical transport). For the
      system service this is the systemd socket-unit ListenStream path (created
      0660 root:whereami so members of the `whereami` group may connect); for the
      home-manager service the daemon self-binds it 0600 under $XDG_RUNTIME_DIR.
    '';
  };

  allowedUsers = lib.mkOption {
    type = lib.types.listOf lib.types.str;
    default = [ ];
    description = ''
      Users granted full daemon control-socket access (SYSTEM module ONLY): they
      are added to the `whereami` group that owns the 0660 socket. Every listed
      user can invoke destructive `whereami wipe --yes`; there is no read-only or
      per-command role. The home-manager module IGNORES this option — its socket
      lives under $XDG_RUNTIME_DIR (0700, user-owned), so its owner has the same
      full capability by construction.
    '';
  };

  wifiInterface = lib.mkOption {
    type = lib.types.str;
    default = "wlan0";
    description = "Wi-Fi interface to scan." + wifiInterfaceExtraDoc;
  };

  dbPath = lib.mkOption {
    type = lib.types.str;
    default = dbPathDefault;
    description = "Path to the SQLite database.";
  };

  credentialsFile = lib.mkOption {
    type = lib.types.nullOr lib.types.str;
    default = credentialsFileDefault;
    description = ''
      Path to a TOML file containing WiGLE credentials.
      On NixOS this is a source read by the root system manager; the daemon
      cannot inspect its mode, so keep it root-owned at 0600 (or 0400 when
      immutable). Home Manager passes this path directly to the daemon, which
      warns when an ordinary file has group or world permission bits.
      Expected format:
        [wigle]
        api_user = "AID..."
        api_key = "..."
    '';
  };

  maxConnections = lib.mkOption {
    type = lib.types.ints.between 1 4096;
    default = 32;
    description = ''
      Maximum number of active control-channel connections, shared by the unix
      socket and optional TCP listener. Excess clients remain in the kernel
      listen backlog until a slot is available.
    '';
  };

  dailyLimit = lib.mkOption {
    # u32, no validate() check: 0 is accepted (effectively disables WiGLE calls).
    type = lib.types.ints.unsigned;
    default = 100;
    description = "Maximum WiGLE API calls per day.";
  };

  scanIntervalFast = lib.mkOption {
    type = lib.types.ints.positive;
    default = 10;
    description = "Scan interval during fast phase (seconds).";
  };

  scanFastDuration = lib.mkOption {
    # u64, no validate() check: 0 is accepted (fast phase lasts 0s -> straight to slow).
    type = lib.types.ints.unsigned;
    default = 60;
    description = "Duration of fast scan phase (seconds).";
  };

  scanIntervalSlow = lib.mkOption {
    type = lib.types.ints.positive;
    default = 60;
    description = "Scan interval during steady phase (seconds).";
  };

  debounceWindow = lib.mkOption {
    type = lib.types.ints.positive;
    default = 10;
    description = "Number of scan samples in debounce ring buffer.";
  };

  debounceThreshold = lib.mkOption {
    # Also constrained <= debounceWindow by a module assertion (nix/assertions.nix).
    type = lib.types.ints.positive;
    default = 5;
    description = "Minimum appearances to be considered stable.";
  };

  topN = lib.mkOption {
    type = lib.types.ints.positive;
    default = 10;
    description = "Number of strongest APs to use for trilateration.";
  };

  mobileSsidDenylist = lib.mkOption {
    type = lib.types.listOf lib.types.str;
    default = [
      "DSB_Wi-Fi"
      ".DSB_Wi-Fi"
      "dsb-mng-wifi"
      "MOVIA*"
      "Bus_WiFi"
      "Bus WiFi"
      "*_iPhone"
      "*_Hotspot"
      "iPhone"
      "AndroidAP*"
    ];
    description = ''
      Scanner-observed SSID patterns classified as mobile access points and
      excluded from positioning. Patterns are anchored globs using
      locale-independent Unicode lowercase comparison: * matches any substring
      and ? matches one Unicode scalar. An empty list disables the heuristic;
      missing, empty, and <hidden> SSIDs never match.
      Current scanner matches are excluded immediately before strongest-AP
      selection. Existing persisted rows themselves are reclassified only
      when they are relearned and upserted, not on every scan or configuration
      reload. Entries must not contain carriage returns or newlines.
    '';
  };

  pendingInterval = lib.mkOption {
    type = lib.types.ints.positive;
    default = 300;
    description = "Seconds between pending queue drain runs.";
  };

  pendingMaxAttempts = lib.mkOption {
    type = lib.types.ints.positive;
    default = 20;
    description = "Drop from pending after this many failed attempts.";
  };

  notFoundTtlDays = lib.mkOption {
    type = lib.types.ints.positive;
    default = 30;
    description = "Days before re-checking a not-found BSSID.";
  };

  addressApprox = lib.mkOption {
    type = lib.types.bool;
    default = false;
    description = "Include approximate street address in locate responses via OSM Nominatim.";
  };

  addressCacheTtlDays = lib.mkOption {
    # i64, validate() bails on <0; 0 is the documented "disable the cache" value.
    type = lib.types.ints.unsigned;
    default = 7;
    description = ''
      TTL for cached reverse-geocoded addresses (days). After this, an entry is
      re-resolved on the next locate at that grid cell. 0 disables the cache.
    '';
  };

  httpTimeoutSecs = lib.mkOption {
    type = lib.types.ints.positive;
    default = 15;
    description = ''
      Total HTTP request timeout for fast endpoints (Apple WPS, WiGLE) in
      seconds. The connect timeout is fixed at 5s.
    '';
  };

  nominatimTimeoutSecs = lib.mkOption {
    type = lib.types.ints.positive;
    default = 30;
    description = ''
      Total HTTP request timeout for the OSM Nominatim endpoint in seconds.
      Nominatim's public instance is frequently slow under load.
    '';
  };

  historyRetentionDays = lib.mkOption {
    # Mirrors the daemon's checked i64 policy; 0 keeps fixes indefinitely.
    type = lib.types.ints.between 0 maxRetentionDays;
    default = 30;
    description = ''
      Days to retain rows in the location-history `fixes` table. Older rows are
      pruned periodically. 0 disables retention (no pruning); fixes are kept
      indefinitely. This governs how long location PII lives on disk.
    '';
  };

  apsRetentionDays = lib.mkOption {
    type = lib.types.ints.between 0 maxRetentionDays;
    default = 30;
    description = ''
      Days to retain resolved access-point rows after `last_seen`. Older rows are
      pruned periodically. 0 disables retention (no pruning); AP rows are kept
      indefinitely. This governs how long resolved, stable cached
      BSSID/SSID/location data lives on disk.
    '';
  };

  historySegmentDistanceM = lib.mkOption {
    type = lib.types.ints.positive;
    default = 100;
    description = ''
      Maximum distance (meters) between consecutive fixes for them to be grouped
      into the same stay-point segment.
    '';
  };

  historySegmentMinDurationSecs = lib.mkOption {
    type = lib.types.ints.positive;
    default = 300;
    description = ''
      Minimum duration (seconds) for a history segment to be reported. Shorter
      runs (e.g. driving past a coffee shop) are dropped.
    '';
  };
}
