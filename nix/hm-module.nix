{ config, lib, pkgs, ... }:

let
  cfg = config.services.whereami;

  execStart = import ./exec-start.nix { inherit lib cfg; };
in {
  options.services.whereami = import ./options.nix {
    inherit lib;
    dbPathDefault = "%h/.local/share/whereami/aps.sqlite";
    credentialsFileDefault = "%h/.config/whereami/config.toml";
  };

  config = lib.mkIf cfg.enable {
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
