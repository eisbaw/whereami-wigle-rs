# `socketFlag` controls whether ExecStart passes `--socket`:
#  - home-manager (self-bind): true  → the daemon binds cfg.socketPath itself.
#  - system (socket activation): false → systemd owns the socket and passes fd 3,
#    so --socket must NOT be given (the fd wins, and passing it would only log a
#    warning). cfg.socketPath is instead the socket unit's ListenStream path.
# `credentialsFromSystemd` selects how --config is delivered (task-0205):
#  - false (home-manager): the user's own systemd manager reads its 0600 config
#    directly, so ExecStart passes the plain cfg.credentialsFile path.
#  - true (system + DynamicUser): the file arrives via LoadCredential (module.nix)
#    because the random DynamicUser UID cannot read a 0600 root:root file; the
#    daemon reads it from systemd's credentials directory instead.
{ lib, cfg, socketFlag ? true, credentialsFromSystemd ? false }:

let
  # ${CREDENTIALS_DIRECTORY} is substituted by systemd IN ExecStart (systemd.exec(5)
  # documents `${CREDENTIALS_DIRECTORY}/mycred`); the "\${" stops Nix interpolating
  # it so systemd receives the literal specifier. The daemon's load_config_file
  # reads any absolute path, so no daemon change is needed.
  configArg =
    if cfg.credentialsFile == null then ""
    else if credentialsFromSystemd then "--config \${CREDENTIALS_DIRECTORY}/config"
    else "--config ${cfg.credentialsFile}";
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
  "--max-connections" (toString cfg.maxConnections)
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
  "--address-cache-ttl-days" (toString cfg.addressCacheTtlDays)
  "--http-timeout-secs" (toString cfg.httpTimeoutSecs)
  "--nominatim-timeout-secs" (toString cfg.nominatimTimeoutSecs)
  "--history-retention-days" (toString cfg.historyRetentionDays)
  "--aps-retention-days" (toString cfg.apsRetentionDays)
  "--history-segment-distance-m" (toString cfg.historySegmentDistanceM)
  "--history-segment-min-duration-secs" (toString cfg.historySegmentMinDurationSecs)
] ++ lib.optional (socketArg != "") socketArg
  ++ lib.optional (bindArg != "") bindArg
  ++ lib.optional (nonlocalArg != "") nonlocalArg
  ++ lib.optional (configArg != "") configArg
  ++ lib.optional (addressArg != "") addressArg)
