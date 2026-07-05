# Evaluate the real shared module options at both sides of the maxConnections
# upper boundary. `tryEval (deepSeq ...)` is intentional: merely constructing
# evalModules is lazy and would not force the option type check.
{ pkgs, lib }:

let
  sharedOptions = { dbPathDefault, credentialsFileDefault, socketPathDefault }:
    import ./options.nix {
      inherit lib dbPathDefault credentialsFileDefault socketPathDefault;
    };

  evaluateMaxConnections = value:
    builtins.tryEval (builtins.deepSeq
      ((lib.evalModules {
        modules = [
          {
            options.services.whereami = sharedOptions {
              dbPathDefault = "/var/lib/whereami/aps.sqlite";
              credentialsFileDefault = null;
              socketPathDefault = "/run/whereami/whereamid.sock";
            };
            config.services.whereami.maxConnections = value;
          }
        ];
      }).config.services.whereami.maxConnections)
      true);

  evaluateRetention = option: value:
    builtins.tryEval (builtins.deepSeq
      ((lib.evalModules {
        modules = [
          {
            options.services.whereami = sharedOptions {
              dbPathDefault = "/var/lib/whereami/aps.sqlite";
              credentialsFileDefault = null;
              socketPathDefault = "/run/whereami/whereamid.sock";
            };
            config.services.whereami.${option} = value;
          }
        ];
      }).config.services.whereami.${option})
      true);

  evaluateMaxTravelSpeed = value:
    builtins.tryEval (builtins.deepSeq
      ((lib.evalModules {
        modules = [
          {
            options.services.whereami = sharedOptions {
              dbPathDefault = "/var/lib/whereami/aps.sqlite";
              credentialsFileDefault = null;
              socketPathDefault = "/run/whereami/whereamid.sock";
            };
            config.services.whereami.maxTravelSpeedKmh = value;
          }
        ];
      }).config.services.whereami.maxTravelSpeedKmh)
      true);

  evaluateAutolocateInterval = value:
    builtins.tryEval (builtins.deepSeq
      ((lib.evalModules {
        modules = [
          {
            options.services.whereami = sharedOptions {
              dbPathDefault = "/var/lib/whereami/aps.sqlite";
              credentialsFileDefault = null;
              socketPathDefault = "/run/whereami/whereamid.sock";
            };
            config.services.whereami.autolocateIntervalSecs = value;
          }
        ];
      }).config.services.whereami.autolocateIntervalSecs)
      true);

  evaluateAutolocateMaxScanAge = value:
    builtins.tryEval (builtins.deepSeq
      ((lib.evalModules {
        modules = [
          {
            options.services.whereami = sharedOptions {
              dbPathDefault = "/var/lib/whereami/aps.sqlite";
              credentialsFileDefault = null;
              socketPathDefault = "/run/whereami/whereamid.sock";
            };
            config.services.whereami.autolocateMaxScanAgeSecs = value;
          }
        ];
      }).config.services.whereami.autolocateMaxScanAgeSecs)
      true);

  optionDefault = defaults: option:
    (lib.evalModules {
      modules = [
        {
          options.services.whereami = sharedOptions defaults;
        }
      ];
    }).config.services.whereami.${option};

  optionDescription = defaults: option:
    (lib.evalModules {
      modules = [
        {
          options.services.whereami = sharedOptions defaults;
        }
      ];
    }).options.services.whereami.${option}.description;

  systemDefaults = {
    dbPathDefault = "/var/lib/whereami/aps.sqlite";
    credentialsFileDefault = null;
    socketPathDefault = "/run/whereami/whereamid.sock";
  };
  homeDefaults = {
    dbPathDefault = "%h/.local/share/whereami/aps.sqlite";
    credentialsFileDefault = "%h/.config/whereami/config.toml";
    socketPathDefault = "%t/whereamid.sock";
  };

  systemCredentialsDescription =
    optionDescription systemDefaults "credentialsFile";
  homeCredentialsDescription = optionDescription homeDefaults "credentialsFile";
  credentialsDescriptionIsCurrent = description:
    lib.hasInfix "[wigle]" description
    && lib.hasInfix "api_user" description
    && lib.hasInfix "api_key" description
    && !(lib.hasInfix "beacondb" (lib.toLower description));

  # Evaluate the real Home Manager module with the smallest surrounding option
  # surface it needs. This pins both sides of the optional Environment contract
  # without duplicating Home Manager's INI renderer (TASK-0234 owns a full HM
  # activation test).
  evaluateHomeModule = { rustLog, mobileSsidDenylist ? optionDefault homeDefaults "mobileSsidDenylist" }:
    (lib.evalModules {
      specialArgs = { inherit pkgs; };
      modules = [
        {
          options = {
            assertions = lib.mkOption {
              type = lib.types.listOf lib.types.unspecified;
              default = [ ];
            };
            warnings = lib.mkOption {
              type = lib.types.listOf lib.types.str;
              default = [ ];
            };
            home.packages = lib.mkOption {
              type = lib.types.listOf lib.types.package;
              default = [ ];
            };
            systemd.user.services = lib.mkOption {
              type = lib.types.attrsOf lib.types.anything;
              default = { };
            };
          };
        }
        ./hm-module.nix
        {
          services.whereami = {
            enable = true;
            package = pkgs.hello;
            inherit rustLog mobileSsidDenylist;
          };
        }
      ];
    }).config;

  homeWithoutRustLog = evaluateHomeModule { rustLog = null; };
  homeWithRustLog = evaluateHomeModule { rustLog = ''whereamid=debug,my crate="trace"%n$HOME''; };
  homeWithMultilineRustLog = evaluateHomeModule { rustLog = "whereamid=debug\nother=trace"; };
  homeWithCarriageReturnRustLog = evaluateHomeModule { rustLog = "whereamid=debug\rother=trace"; };
  homeWithMultilineMobilePattern = evaluateHomeModule {
    rustLog = null;
    mobileSsidDenylist = [ "MOVIA*\nBus*" ];
  };
  homeWithCarriageReturnMobilePattern = evaluateHomeModule {
    rustLog = null;
    mobileSsidDenylist = [ "MOVIA*\rBus*" ];
  };
  homeServiceWithoutRustLog =
    homeWithoutRustLog.systemd.user.services.whereamid.Service;
  homeServiceWithRustLog = homeWithRustLog.systemd.user.services.whereamid.Service;

  hasFailedRustLogAssertion = evaluated:
    lib.any (entry:
      !entry.assertion && lib.hasInfix "single-line RUST_LOG" entry.message
    ) evaluated.assertions;

  hasFailedMobilePatternAssertion = evaluated:
    lib.any (entry:
      !entry.assertion && lib.hasInfix "mobileSsidDenylist entries must be single-line" entry.message
    ) evaluated.assertions;

  expectedMobileSsidDenylist = [
    "DSB_Wi-Fi"
    ".DSB_Wi-Fi"
    "dsb-mng-wifi"
    "MOVIA*"
    "Bus_WiFi"
    "Bus WiFi"
    "*_iPhone"
    "*_Hotspot"
    "iPhone"
    "AndroidAP*"
  ];

  atUpperBound = evaluateMaxConnections 4096;
  aboveUpperBound = evaluateMaxConnections 4097;
  retentionOptions = [ "historyRetentionDays" "apsRetentionDays" ];
  retentionBoundsHold = lib.all (option:
    (evaluateRetention option 0).success
    && (evaluateRetention option 365000).success
    && !(evaluateRetention option (-1)).success
    && !(evaluateRetention option 365001).success
  ) retentionOptions;
  speedBoundsHold =
    (evaluateMaxTravelSpeed 0.5).success
    && (evaluateMaxTravelSpeed 250).success
    && !(evaluateMaxTravelSpeed 0).success
    && !(evaluateMaxTravelSpeed (-1)).success;
  autolocateBoundsHold =
    (evaluateAutolocateInterval 0).success
    && (evaluateAutolocateInterval 1).success
    && (evaluateAutolocateInterval 31536000).success
    && !(evaluateAutolocateInterval (-1)).success
    && !(evaluateAutolocateInterval 31536001).success;
  autolocateMaxScanAgeBoundsHold =
    (evaluateAutolocateMaxScanAge 0).success
    && (evaluateAutolocateMaxScanAge 1).success
    && (evaluateAutolocateMaxScanAge 31536000).success
    && !(evaluateAutolocateMaxScanAge (-1)).success
    && !(evaluateAutolocateMaxScanAge 31536001).success;
in
assert atUpperBound.success && atUpperBound.value;
assert !aboveUpperBound.success;
assert retentionBoundsHold;
assert speedBoundsHold;
assert autolocateBoundsHold;
assert autolocateMaxScanAgeBoundsHold;
assert optionDefault systemDefaults "dbPath" == "/var/lib/whereami/aps.sqlite";
assert optionDefault homeDefaults "dbPath" == "%h/.local/share/whereami/aps.sqlite";
assert optionDefault systemDefaults "historyRetentionDays" == 30;
assert optionDefault systemDefaults "apsRetentionDays" == 30;
assert optionDefault homeDefaults "historyRetentionDays" == 30;
assert optionDefault homeDefaults "apsRetentionDays" == 30;
assert optionDefault systemDefaults "maxTravelSpeedKmh" == 250;
assert optionDefault homeDefaults "maxTravelSpeedKmh" == 250;
assert optionDefault systemDefaults "autolocateIntervalSecs" == 120;
assert optionDefault homeDefaults "autolocateIntervalSecs" == 120;
assert optionDefault systemDefaults "autolocateMaxScanAgeSecs" == 180;
assert optionDefault homeDefaults "autolocateMaxScanAgeSecs" == 180;
assert optionDefault systemDefaults "mobileSsidDenylist" == expectedMobileSsidDenylist;
assert optionDefault homeDefaults "mobileSsidDenylist" == expectedMobileSsidDenylist;
assert optionDefault systemDefaults "rustLog" == null;
assert optionDefault homeDefaults "rustLog" == null;
assert credentialsDescriptionIsCurrent systemCredentialsDescription;
assert credentialsDescriptionIsCurrent homeCredentialsDescription;
assert systemCredentialsDescription == homeCredentialsDescription;
assert !(homeServiceWithoutRustLog ? Environment);
assert homeServiceWithRustLog.Environment == [
  "\"RUST_LOG=whereamid=debug,my crate=\\\"trace\\\"%%n$HOME\""
];
assert lib.all (entry: entry.assertion) homeWithRustLog.assertions;
assert hasFailedRustLogAssertion homeWithMultilineRustLog;
assert hasFailedRustLogAssertion homeWithCarriageReturnRustLog;
assert hasFailedMobilePatternAssertion homeWithMultilineMobilePattern;
assert hasFailedMobilePatternAssertion homeWithCarriageReturnMobilePattern;
pkgs.runCommand "whereami-option-bounds" { } ''
  echo "maxConnections bounds OK: 4096 accepted, 4097 rejected"
  echo "retention bounds OK: 0..365000 accepted, outside rejected"
  echo "maxTravelSpeedKmh bounds OK: positive numbers accepted, zero/negative rejected"
  echo "autolocateIntervalSecs bounds OK: 0..31536000 accepted, outside rejected"
  echo "autolocateMaxScanAgeSecs bounds OK: 0..31536000 accepted, outside rejected"
  echo "system/home defaults OK: dbPath, retention=30, maxTravelSpeedKmh=250, autolocateIntervalSecs=120, autolocateMaxScanAgeSecs=180, rustLog=null"
  echo "shared credentialsFile docs OK: WiGLE keys present, BeaconDB absent"
  echo "home-manager RUST_LOG encoding OK: optional, quoted, percent-safe, CR/LF rejected"
  echo "mobile SSID defaults and CR/LF rejection OK"
  touch "$out"
''
