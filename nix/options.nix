{ lib, dbPathDefault, credentialsFileDefault }:

{
  enable = lib.mkEnableOption "whereami Wi-Fi geolocation daemon";

  package = lib.mkOption {
    type = lib.types.package;
    description = "The whereami package to use.";
  };

  bind = lib.mkOption {
    type = lib.types.str;
    default = "127.0.0.1:4747";
    description = "TCP bind address for whereamid.";
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
