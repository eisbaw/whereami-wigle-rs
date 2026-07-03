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
]
