{ config, lib, pkgs, ... }:

let
  cfg = config.services.whereami;

  # The user service self-binds its own socket (socketFlag = true → ExecStart
  # passes --socket cfg.socketPath). No group is needed: %t is $XDG_RUNTIME_DIR
  # (/run/user/<uid>, mode 0700, user-owned), so the socket is single-user by
  # construction and the daemon tightens it to 0600.
  execStart = import ./exec-start.nix { inherit lib cfg; socketFlag = true; };
in {
  options.services.whereami = import ./options.nix {
    inherit lib;
    dbPathDefault = "%h/.local/share/whereami/aps.sqlite";
    credentialsFileDefault = "%h/.config/whereami/config.toml";
    socketPathDefault = "%t/whereamid.sock";
  };

  config = lib.mkIf cfg.enable {
    # Cross-field option invariants mirroring Args::validate() (task-0207). Shared
    # with the NixOS module. home-manager implements `assertions` like NixOS.
    assertions = import ./assertions.nix { inherit lib cfg; };

    home.packages = [ cfg.package ];

    systemd.user.services.whereamid = {
      Unit = {
        Description = "whereami Wi-Fi geolocation daemon";
        After = [ "network.target" ];
      };
      Service = {
        Type = "simple";
        ExecStart = execStart;
        Restart = "on-failure";
        RestartSec = 5;
        # task-0197: force a 0077 umask so the DB under %h/.local/share/whereami
        # is created 0600, not world-readable 0644. There is no StateDirectory
        # for user units (no StateDirectoryMode to set); the daemon creates the
        # directory itself at mode 0700 and tightens the DB file at runtime.
        UMask = "0077";
      };
      Install = {
        WantedBy = [ "default.target" ];
      };
    };
  };
}
