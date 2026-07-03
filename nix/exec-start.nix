# `socketFlag` controls whether ExecStart passes `--socket`:
#  - home-manager (self-bind): true  → the daemon binds cfg.socketPath itself.
#  - system (socket activation): false → systemd owns the socket and passes fd 3,
#    so --socket must NOT be given (the fd wins, and passing it would only log a
#    warning). cfg.socketPath is instead the socket unit's ListenStream path.
{ lib, cfg, socketFlag ? true }:

let
  configArg = lib.optionalString (cfg.credentialsFile != null)
    "--config ${cfg.credentialsFile}";
  addressArg = lib.optionalString cfg.addressApprox "--address-approx";
  socketArg = lib.optionalString socketFlag "--socket ${cfg.socketPath}";
  # TCP is opt-in (cfg.bind == null → unix socket only). A non-loopback bind
  # additionally needs --allow-nonlocal-bind, which the daemon's own validate()
  # enforces; we only forward the flag alongside an actual --bind.
  bindArg = lib.optionalString (cfg.bind != null) "--bind ${cfg.bind}";
  nonlocalArg = lib.optionalString (cfg.bind != null && cfg.allowNonlocalBind)
    "--allow-nonlocal-bind";
in
lib.concatStringsSep " " ([
  "${cfg.package}/bin/whereamid"
  "--db" cfg.dbPath
  "--interface" cfg.wifiInterface
  "--scan-interval-fast" (toString cfg.scanIntervalFast)
  "--scan-fast-duration" (toString cfg.scanFastDuration)
  "--scan-interval-slow" (toString cfg.scanIntervalSlow)
  "--debounce-window" (toString cfg.debounceWindow)
  "--debounce-threshold" (toString cfg.debounceThreshold)
  "--top-n" (toString cfg.topN)
  "--pending-interval" (toString cfg.pendingInterval)
  "--pending-max-attempts" (toString cfg.pendingMaxAttempts)
  "--daily-limit" (toString cfg.dailyLimit)
  "--not-found-ttl-days" (toString cfg.notFoundTtlDays)
] ++ lib.optional (socketArg != "") socketArg
  ++ lib.optional (bindArg != "") bindArg
  ++ lib.optional (nonlocalArg != "") nonlocalArg
  ++ lib.optional (configArg != "") configArg
  ++ lib.optional (addressArg != "") addressArg)
