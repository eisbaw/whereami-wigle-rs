{ config, lib, pkgs, ... }:

let
  cfg = config.services.whereami;

  # The system service uses systemd socket activation (fd 3), so ExecStart must
  # NOT pass --socket (socketFlag = false); cfg.socketPath is instead the socket
  # unit's ListenStream path.
  execStart = import ./exec-start.nix { inherit lib cfg; socketFlag = false; };
in {
  options.services.whereami = import ./options.nix {
    inherit lib;
    dbPathDefault = "/var/lib/whereami/aps.sqlite";
    credentialsFileDefault = null;
    socketPathDefault = "/run/whereami/whereamid.sock";
  };

  config = lib.mkIf cfg.enable {
    # Cross-field option invariants mirroring Args::validate() (task-0207), so a
    # bad combination fails `nixos-rebuild` rather than crash-looping the daemon.
    assertions = import ./assertions.nix { inherit lib cfg; };

    # The `whereami` group owns the 0660 control socket; its members are the
    # users allowed to query the daemon (task-0196 / ADR-003). Empty by default,
    # so only root can connect until the owner opts users in via allowedUsers.
    users.groups.whereami.members = cfg.allowedUsers;

    # Socket activation is what makes the DynamicUser daemon reachable by the
    # owner: systemd creates the socket as root:whereami mode 0660 BEFORE the
    # (random-UID) service starts and hands it fd 3, so members of `whereami` can
    # connect while the daemon needs no fixed UID. The socket's filesystem
    # permissions ARE the access control (ADR-003).
    systemd.sockets.whereamid = {
      description = "whereami control socket";
      wantedBy = [ "sockets.target" ];
      socketConfig = {
        ListenStream = cfg.socketPath;
        SocketMode = "0660";
        SocketUser = "root";
        SocketGroup = "whereami";
      };
    };

    systemd.services.whereamid = {
      description = "whereami Wi-Fi geolocation daemon";
      after = [ "network.target" "NetworkManager.service" "whereamid.socket" ];
      requires = [ "whereamid.socket" ];
      # Scanning must run 24/7 (continuously build the AP cache + history), so
      # the service is always started at boot rather than purely on-demand — it
      # merely USES the socket-activated fd.
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
