{ config, lib, pkgs, ... }:

let
  cfg = config.services.whereami;
  whereamid = pkgs.callPackage ./package.nix {};
in {
  options.services.whereami = {
    enable = lib.mkEnableOption "whereami Wi-Fi geolocation daemon";

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
      default = "/var/lib/whereami/aps.sqlite";
      description = "Path to the SQLite database.";
    };

    credentialsFile = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = ''
        Path to a TOML file containing WiGLE credentials.
        This file is NOT copied to the nix store — it is read at runtime.
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
  };

  config = lib.mkIf cfg.enable {
    systemd.services.whereamid = {
      description = "whereami Wi-Fi geolocation daemon";
      after = [ "network.target" "NetworkManager.service" ];
      wantedBy = [ "multi-user.target" ];

      serviceConfig = {
        ExecStart = let
          configArg = lib.optionalString (cfg.credentialsFile != null)
            "--config ${cfg.credentialsFile}";
          addressArg = lib.optionalString cfg.addressApprox "--address-approx";
        in lib.concatStringsSep " " ([
          "${whereamid}/bin/whereamid"
          "--bind" cfg.bind
          "--db" cfg.dbPath
          "--interface" cfg.wifiInterface
          "--scan-interval-fast" (toString cfg.scanIntervalFast)
          "--scan-fast-duration" (toString cfg.scanFastDuration)
          "--scan-interval-slow" (toString cfg.scanIntervalSlow)
          "--debounce-window" (toString cfg.debounceWindow)
          "--debounce-threshold" (toString cfg.debounceThreshold)
          "--top-n" (toString cfg.topN)
          "--pending-interval" (toString cfg.pendingInterval)
          "--pending-max-attempts" (toString cfg.pendingMaxAttempts)
          "--daily-limit" (toString cfg.dailyLimit)
          "--not-found-ttl-days" (toString cfg.notFoundTtlDays)
        ] ++ lib.optional (configArg != "") configArg
          ++ lib.optional (addressArg != "") addressArg);

        DynamicUser = true;
        StateDirectory = "whereami";
        AmbientCapabilities = [ "CAP_NET_ADMIN" ];
        CapabilityBoundingSet = [ "CAP_NET_ADMIN" ];

        # Hardening
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectControlGroups = true;
        RestrictAddressFamilies = [ "AF_INET" "AF_INET6" "AF_NETLINK" "AF_UNIX" ];
        RestrictNamespaces = true;
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
        RestrictRealtime = true;
        SystemCallFilter = [ "@system-service" "@network-io" ];
      };
    };
  };
}
