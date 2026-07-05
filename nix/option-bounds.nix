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

  atUpperBound = evaluateMaxConnections 4096;
  aboveUpperBound = evaluateMaxConnections 4097;
in
assert atUpperBound.success && atUpperBound.value;
assert !aboveUpperBound.success;
pkgs.runCommand "whereami-option-bounds" { } ''
  echo "maxConnections bounds OK: 4096 accepted, 4097 rejected"
  touch "$out"
''
