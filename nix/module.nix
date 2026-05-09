{ config, lib, pkgs, ... }:

let
  cfg = config.services.whereami;

  execStart = import ./exec-start.nix { inherit lib cfg; };
in {
  options.services.whereami = import ./options.nix {
    inherit lib;
    dbPathDefault = "/var/lib/whereami/aps.sqlite";
    credentialsFileDefault = null;
  };

  config = lib.mkIf cfg.enable {
    systemd.services.whereamid = {
      description = "whereami Wi-Fi geolocation daemon";
      after = [ "network.target" "NetworkManager.service" ];
      wantedBy = [ "multi-user.target" ];

      serviceConfig = {
        ExecStart = execStart;
        DynamicUser = true;
        StateDirectory = "whereami";
        AmbientCapabilities = [ "CAP_NET_ADMIN" ];
        CapabilityBoundingSet = [ "CAP_NET_ADMIN" ];
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
