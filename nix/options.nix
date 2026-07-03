{ lib, dbPathDefault, credentialsFileDefault, socketPathDefault }:

{
  enable = lib.mkEnableOption "whereami Wi-Fi geolocation daemon";

  package = lib.mkOption {
    type = lib.types.package;
    description = "The whereami package to use.";
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
      Users permitted to query the daemon (SYSTEM module ONLY): they are added to
      the `whereami` group that owns the 0660 control socket. The home-manager
      module IGNORES this option — its socket lives under $XDG_RUNTIME_DIR
      (0700, user-owned), so it is single-user by construction.
    '';
  };

  wifiInterface = lib.mkOption {
    type = lib.types.str;
    default = "wlan0";
    description = "Wi-Fi interface to scan.";
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
      Expected format:
        [wigle]
        api_user = "AID..."
        api_key = "..."

        [beacondb]
        enabled = true
    '';
  };

  dailyLimit = lib.mkOption {
    type = lib.types.int;
    default = 100;
    description = "Maximum WiGLE API calls per day.";
  };

  scanIntervalFast = lib.mkOption {
    type = lib.types.int;
    default = 10;
    description = "Scan interval during fast phase (seconds).";
  };

  scanFastDuration = lib.mkOption {
    type = lib.types.int;
    default = 60;
    description = "Duration of fast scan phase (seconds).";
  };

  scanIntervalSlow = lib.mkOption {
    type = lib.types.int;
    default = 60;
    description = "Scan interval during steady phase (seconds).";
  };

  debounceWindow = lib.mkOption {
    type = lib.types.int;
    default = 10;
    description = "Number of scan samples in debounce ring buffer.";
  };

  debounceThreshold = lib.mkOption {
    type = lib.types.int;
    default = 5;
    description = "Minimum appearances to be considered stable.";
  };

  topN = lib.mkOption {
    type = lib.types.int;
    default = 10;
    description = "Number of strongest APs to use for trilateration.";
  };

  pendingInterval = lib.mkOption {
    type = lib.types.int;
    default = 300;
    description = "Seconds between pending queue drain runs.";
  };

  pendingMaxAttempts = lib.mkOption {
    type = lib.types.int;
    default = 20;
    description = "Drop from pending after this many failed attempts.";
  };

  notFoundTtlDays = lib.mkOption {
    type = lib.types.int;
    default = 30;
    description = "Days before re-checking a not-found BSSID.";
  };

  addressApprox = lib.mkOption {
    type = lib.types.bool;
    default = false;
    description = "Include approximate street address in locate responses via OSM Nominatim.";
  };
}
