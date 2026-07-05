# Shared module assertions for services.whereami, imported by BOTH nix/module.nix
# (NixOS) and nix/hm-module.nix (home-manager). Single source of truth for
# cross-field invariants that a per-option `type` cannot express (task-0207).
#
# These mirror the cross-field checks in whereamid's Args::validate() (config.rs)
# so a bad combination fails module evaluation instead of crash-looping the
# daemon at startup. Single-field bounds live on the option `type` in options.nix.
{ lib, cfg }:

[
  {
    # config.rs Args::validate(): `debounce_threshold > debounce_window` bails.
    # A threshold larger than the ring-buffer window can never be reached, so the
    # daemon would exit non-zero on every start.
    assertion = cfg.debounceThreshold <= cfg.debounceWindow;
    message =
      "services.whereami.debounceThreshold (${toString cfg.debounceThreshold}) must be "
      + "<= services.whereami.debounceWindow (${toString cfg.debounceWindow}): a threshold "
      + "above the ring-buffer window can never be met and whereamid's Args::validate() "
      + "rejects it at startup.";
  }
  {
    # systemd environment assignments are line-oriented. Reject CR/LF here
    # instead of relying on the NixOS and Home Manager unit renderers to agree
    # on how (or whether) an embedded line break is escaped.
    assertion =
      cfg.rustLog == null
      || (!lib.hasInfix "\n" cfg.rustLog && !lib.hasInfix "\r" cfg.rustLog);
    message =
      "services.whereami.rustLog must be a single-line RUST_LOG filter; "
      + "carriage returns and newlines are not supported by systemd Environment=.";
  }
  {
    # Keep the comma-separated ExecStart argument single-line. The systemd
    # encoder safely handles spaces/quotes/$/%/semicolon, but CR/LF make the
    # operator-facing module value and generated unit unnecessarily ambiguous.
    assertion = lib.all (pattern:
      !lib.hasInfix "\n" pattern && !lib.hasInfix "\r" pattern
    ) cfg.mobileSsidDenylist;
    message =
      "services.whereami.mobileSsidDenylist entries must be single-line patterns; "
      + "carriage returns and newlines are not supported.";
  }
]
