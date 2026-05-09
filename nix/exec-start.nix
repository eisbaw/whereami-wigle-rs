{ lib, cfg }:

let
  configArg = lib.optionalString (cfg.credentialsFile != null)
    "--config ${cfg.credentialsFile}";
  addressArg = lib.optionalString cfg.addressApprox "--address-approx";
in
lib.concatStringsSep " " ([
  "${cfg.package}/bin/whereamid"
  "--bind" cfg.bind
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
] ++ lib.optional (configArg != "") configArg
  ++ lib.optional (addressArg != "") addressArg)
