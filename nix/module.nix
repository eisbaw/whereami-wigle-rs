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
        # task-0197: the state dir and DB hold Wi-Fi location history. Make the
        # dir owner-only and force a 0077 umask so aps.sqlite (and its WAL
        # sidecars) are created 0600, not the default 0644. DynamicUser already
        # isolates state under /var/lib/private (0700); these make the intent
        # explicit and also cover a non-DynamicUser reconfiguration. The daemon
        # enforces the same modes at runtime as a belt-and-braces backstop.
        StateDirectoryMode = "0700";
        UMask = "0077";
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
