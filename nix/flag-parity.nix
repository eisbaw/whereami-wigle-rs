# Flag-parity guard (task-0206 AC#3): fail the build when whereamid's clap flags
# and the flags emitted by exec-start.nix drift apart.
#
# This is a REAL help-text diff, not a hardcoded list: it compares
#   (a) the long flags `whereamid --help` actually accepts (the built binary is
#       the source of truth), against
#   (b) the `--flag` tokens exec-start.nix emits for a FULLY-POPULATED cfg
#       (socketFlag = true so --socket fires; bind/allowNonlocalBind/credentials
#       File/addressApprox set so every conditional flag fires).
# A new clap flag with no module option -> appears in (a) only -> FAIL. A removed
# clap flag still emitted by exec-start -> appears in (b) only -> FAIL. So the
# guard bites in BOTH directions (a hardcoded list only bites on removal).
#
# Allowlist: NONE. A fully-populated cfg with socketFlag = true emits every flag
# the binary defines, so exact set-equality holds (modulo clap's auto
# --help/--version, which are stripped below). --socket is emitted here even
# though the SYSTEM module omits it (socket activation) — the guard checks what
# CAN be emitted, not any single deployment.
{ pkgs, lib, package }:

let
  # Every option exec-start.nix reads, set so all conditional flags fire. Values
  # are dummies (only the flag NAMES matter); none start with "--" so they are
  # never mistaken for flags by the prefix filter below.
  # DELIBERATE duplication of the option list: if exec-start.nix grows a cfg
  # field missing here, the import below throws `attribute missing` — a loud
  # eval error, never a silent pass.
  cfg = {
    package = "/flag-parity-dummy";
    dbPath = "/flag-parity/aps.sqlite";
    wifiInterface = "wlan-parity";
    maxConnections = 1;
    scanIntervalFast = 1;
    scanFastDuration = 1;
    scanIntervalSlow = 1;
    debounceWindow = 1;
    debounceThreshold = 1;
    topN = 1;
    pendingInterval = 1;
    pendingMaxAttempts = 1;
    dailyLimit = 1;
    notFoundTtlDays = 1;
    socketPath = "/flag-parity/whereamid.sock";
    bind = "127.0.0.1:9999";
    allowNonlocalBind = true;
    credentialsFile = "/flag-parity/config.toml";
    addressApprox = true;
    addressCacheTtlDays = 1;
    httpTimeoutSecs = 1;
    nominatimTimeoutSecs = 1;
    historyRetentionDays = 1;
    historySegmentDistanceM = 1;
    historySegmentMinDurationSecs = 1;
  };

  execStart = import ./exec-start.nix { inherit lib cfg; socketFlag = true; };

  # Split the space-joined ExecStart and keep the tokens that are flags.
  emittedFlags = lib.unique
    (builtins.filter (s: lib.hasPrefix "--" s) (lib.splitString " " execStart));
in
pkgs.runCommand "whereami-flag-parity"
{
  nativeBuildInputs = [ package ];
  emitted = lib.concatStringsSep "\n" (lib.sort (a: b: a < b) emittedFlags);
} ''
  set -euo pipefail
  printf '%s\n' "$emitted" | sort -u > emitted-flags

  # Long flags the binary actually accepts. Restrict to option-HEADER lines
  # (clap indents them; the first non-space char is '-'), so a flag NAMED in a
  # description (e.g. --bind's help mentions --allow-nonlocal-bind) is not
  # miscounted. Then drop clap's auto-generated --help/--version.
  whereamid --help \
    | grep -E '^[[:space:]]+-' \
    | grep -oE -- '--[a-z][a-z0-9-]*' \
    | grep -vxE -- '--(help|version)' \
    | sort -u > help-flags

  if ! diff -u help-flags emitted-flags > parity.diff; then
    {
      echo "FLAG PARITY DRIFT between 'whereamid --help' and nix/exec-start.nix:"
      cat parity.diff
      echo ""
      echo "  '-' lines: a clap flag with NO services.whereami.* option/emission"
      echo "             (add the option in options.nix + emit it in exec-start.nix)."
      echo "  '+' lines: exec-start emits a flag the binary does not accept"
      echo "             (remove it, or the clap flag was renamed/deleted)."
    } >&2
    exit 1
  fi

  echo "flag parity OK: $(wc -l < help-flags) flags match between --help and exec-start.nix"
  touch $out
''
