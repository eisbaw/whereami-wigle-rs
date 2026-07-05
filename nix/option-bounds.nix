# Evaluate the real shared module options at both sides of the maxConnections
# upper boundary. `tryEval (deepSeq ...)` is intentional: merely constructing
# evalModules is lazy and would not force the option type check.
{ pkgs, lib }:

let
  evaluateMaxConnections = value:
    builtins.tryEval (builtins.deepSeq
      ((lib.evalModules {
        modules = [
          {
            options.services.whereami = import ./options.nix {
              inherit lib;
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
            options.services.whereami = import ./options.nix {
              inherit lib;
              dbPathDefault = "/var/lib/whereami/aps.sqlite";
              credentialsFileDefault = null;
              socketPathDefault = "/run/whereami/whereamid.sock";
            };
            config.services.whereami.${option} = value;
          }
        ];
      }).config.services.whereami.${option})
      true);

  atUpperBound = evaluateMaxConnections 4096;
  aboveUpperBound = evaluateMaxConnections 4097;
  retentionOptions = [ "historyRetentionDays" "apsRetentionDays" ];
  retentionBoundsHold = lib.all (option:
    (evaluateRetention option 0).success
    && (evaluateRetention option 365000).success
    && !(evaluateRetention option (-1)).success
    && !(evaluateRetention option 365001).success
  ) retentionOptions;
in
assert atUpperBound.success && atUpperBound.value;
assert !aboveUpperBound.success;
assert retentionBoundsHold;
pkgs.runCommand "whereami-option-bounds" { } ''
  echo "maxConnections bounds OK: 4096 accepted, 4097 rejected"
  echo "retention bounds OK: 0..365000 accepted, outside rejected"
  touch "$out"
''
