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
      };
      Install = {
        WantedBy = [ "default.target" ];
      };
    };
  };
}
