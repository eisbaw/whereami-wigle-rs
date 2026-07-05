{ config, lib, pkgs, ... }:

let
  cfg = config.services.whereami;

  # The system service uses systemd socket activation (fd 3), so ExecStart must
  # NOT pass --socket (socketFlag = false); cfg.socketPath is instead the socket
  # unit's ListenStream path.
  execStart = import ./exec-start.nix {
    inherit lib cfg;
    socketFlag = false;
    # DynamicUser's random UID cannot read a 0600 root config, so secrets are
    # delivered via systemd LoadCredential and --config points at the credentials
    # directory rather than cfg.credentialsFile (task-0205).
    credentialsFromSystemd = true;
  };
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

      # task-0204: bound the restart backoff. StartLimit* live at the UNIT level
      # in systemd (NixOS exposes them as these top-level attrs, NOT under
      # serviceConfig — putting them in serviceConfig silently no-ops). If the
      # daemon fails to start more than `Burst` times within `IntervalSec`,
      # systemd stops retrying and leaves the unit `failed` (so a hard-failing
      # binary cannot hot-loop). 5 starts / 60s is well clear of 5 x RestartSec
      # (10s), so the limit is reached deterministically for a persistently
      # failing config while a single SIGKILL restart never approaches it.
      startLimitIntervalSec = 60;
      startLimitBurst = 5;

      serviceConfig = {
        ExecStart = execStart;
        # task-0204: without this the systemd default Restart=no applies, so a
        # single fatal error (panic, killed process, failed DB open) leaves the
        # daemon permanently dead until manual restart or reboot — worse than the
        # hm unit, which already sets Restart=on-failure. on-failure covers a
        # non-zero exit and signal death (incl. SIGKILL) but NOT a clean exit(0).
        # task-0233 (done): a runtime failure of the OPT-IN TCP `bind` no longer
        # self-exits — the daemon degrades to serving the mandatory unix socket
        # and warns loudly (server.rs run_server, pinned by tests/shutdown.rs),
        # so a busy opt-in port can neither take the daemon down nor (had the
        # exit been made non-zero) crash-loop it to start-limit-hit. The unix
        # socket's own bind failure stays fatal (non-zero, and thus retried).
        # task-0241: if the running server task later returns or panics, main
        # completes its orderly drain and then exits non-zero, so on-failure
        # restarts it instead of accepting a dead daemon as successful.
        # RestartSec is kept small so recovery is fast and the start-limit above
        # is reached quickly for a persistently-failing config.
        Restart = "on-failure";
        RestartSec = 2;
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
        PrivateDevices = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectControlGroups = true;
        ProtectProc = "invisible";
        ProcSubset = "pid";
        ProtectHostname = true;
        ProtectClock = true;
        RestrictAddressFamilies = [ "AF_INET" "AF_INET6" "AF_NETLINK" "AF_UNIX" ];
        RestrictNamespaces = true;
        RestrictSUIDSGID = true;
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
        RestrictRealtime = true;
        SystemCallArchitectures = "native";
        SystemCallFilter = [ "@system-service" "@network-io" ];
      } // lib.optionalAttrs (cfg.credentialsFile != null) {
        # task-0205: deliver the WiGLE secrets to the DynamicUser via systemd
        # credentials. The manager (root) reads the source — which may stay
        # 0600 root:root — and exposes a 0400 copy under $CREDENTIALS_DIRECTORY;
        # ExecStart reads --config ${CREDENTIALS_DIRECTORY}/config. The source
        # file's own mode is never widened (no world-readable secret required).
        LoadCredential = [ "config:${cfg.credentialsFile}" ];
      };
    };
  };
}
