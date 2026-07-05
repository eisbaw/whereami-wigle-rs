{ config, lib, pkgs, ... }:

let
  cfg = config.services.whereami;

  # The user service self-binds its own socket (socketFlag = true → ExecStart
  # passes --socket cfg.socketPath). No group is needed: %t is $XDG_RUNTIME_DIR
  # (/run/user/<uid>, mode 0700, user-owned), so the socket is single-user by
  # construction and the daemon tightens it to 0600.
  execStart = import ./exec-start.nix { inherit lib cfg; socketFlag = true; };

  # Home Manager's Service.Environment option is rendered verbatim as an INI
  # value. Quote the complete NAME=value assignment (JSON string escaping is
  # compatible with systemd.syntax) and double `%` so systemd does not expand
  # specifiers in an operator-supplied RUST_LOG filter. `$` is not expanded by
  # Environment=. Shared assertions reject the unsupported CR/LF case.
  environmentAssignment = name: value:
    builtins.toJSON ("${name}=" + lib.replaceStrings [ "%" ] [ "%%" ] value);

  # task-0208: the USER service can only scan Wi-Fi via NetworkManager. The
  # scanner (scanner.rs wifi_scan) tries nmcli then falls back to `iw scan`, but
  # the iw path needs CAP_NET_ADMIN — a systemd --user unit cannot be granted
  # AmbientCapabilities, so iw can never succeed here. On hosts where
  # NetworkManager does not manage the device the daemon silently never scans.
  # Surfaced BOTH as a build-time warning and in the wifiInterface option docs
  # (single source of truth for the text). There is no hm VM test yet (task-0234).
  nmOnlyNote =
    "whereami home-manager (user service): Wi-Fi scanning requires NetworkManager"
    + " to be actively managing the interface. The scanner uses nmcli and falls"
    + " back to `iw scan`, but the iw path needs CAP_NET_ADMIN, which a systemd"
    + " --user unit cannot be granted (no AmbientCapabilities). On hosts where"
    + " NetworkManager does not manage the Wi-Fi device (iwd, bare wpa_supplicant,"
    + " headless), the daemon can never scan and only logs 'wifi scan failed'.";
in {
  options.services.whereami = import ./options.nix {
    inherit lib;
    dbPathDefault = "%h/.local/share/whereami/aps.sqlite";
    credentialsFileDefault = "%h/.config/whereami/config.toml";
    socketPathDefault = "%t/whereamid.sock";
    # Append the NetworkManager-only caveat to the wifiInterface docs (task-0208).
    wifiInterfaceExtraDoc = "\n\n" + nmOnlyNote;
  };

  config = lib.mkIf cfg.enable {
    # Cross-field option invariants mirroring Args::validate() (task-0207). Shared
    # with the NixOS module. home-manager implements `assertions` like NixOS.
    assertions = import ./assertions.nix { inherit lib cfg; };

    # task-0208: make the NetworkManager-only scanning dependency visible at
    # `home-manager switch` time, not just discoverable in the option docs. This
    # daemon's whole product is a location database, so a service that silently
    # never scans is the failure mode most worth surfacing loudly.
    warnings = [ nmOnlyNote ];

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
      } // lib.optionalAttrs (cfg.rustLog != null) {
        Environment = [ (environmentAssignment "RUST_LOG" cfg.rustLog) ];
      };
      Install = {
        WantedBy = [ "default.target" ];
      };
    };
  };
}
