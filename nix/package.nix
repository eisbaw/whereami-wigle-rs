{ lib
, makeWrapper
, pkg-config
, openssl
, sqlite
, iw
, networkmanager
, craneLib
  # The flake's own git revision, threaded in from flake.nix (self.shortRev).
  # This REPLACES an in-package `builtins.fetchGit { url = ./..; }`, which
  # cannot work under `nix flake check` pure evaluation: the flake source is
  # copied to a non-git /nix/store path, so fetchGit sees an "unlocked input"
  # and aborts. The flake already knows its own rev purely, so we pass it in.
  # Defaults to "unknown" so a bare `callPackage` (outside the flake) still
  # builds; build.rs makes the same fallback at compile time. task-0209.
, gitRev ? "unknown"
}:

let
  cargoToml = builtins.fromTOML (builtins.readFile ../whereamid/Cargo.toml);
  src = craneLib.cleanCargoSource ./..;

  commonArgs = {
    pname = "whereamid";
    version = cargoToml.package.version;
    inherit src;
    strictDeps = true;

    nativeBuildInputs = [
      makeWrapper
      pkg-config
    ];

    buildInputs = [
      openssl
      sqlite
    ];
  };

  # Phase 1: deps only (cached until Cargo.lock changes). SHARED by the package
  # build AND every check below (clippy/test), so nothing recompiles the
  # dependency tree three times. task-0209.
  cargoArtifacts = craneLib.buildDepsOnly commonArgs;

  # Phase 2: workspace code (reuses cached deps). Contents kept byte-identical
  # to the pre-0209 package: same commonArgs, cargoArtifacts, preBuild,
  # postInstall and meta — only the gitRev SOURCE changed (fetchGit -> arg).
  package = craneLib.buildPackage (commonArgs // {
    inherit cargoArtifacts;

    preBuild = ''
      echo "${gitRev}" > whereamid/GIT_REV
      echo "${gitRev}" > whereami-client/GIT_REV
    '';

    postInstall = ''
      wrapProgram $out/bin/whereamid \
        --prefix PATH : ${lib.makeBinPath [ iw networkmanager ]}
    '';

    meta = with lib; {
      description = "Wi-Fi geolocation daemon";
      license = licenses.mit;
      mainProgram = "whereamid";
    };
  });
in
{
  inherit package;

  # `nix flake check` wires these into `checks.<system>`. They reuse the shared
  # cargoArtifacts, so `nix flake check` builds deps once and then runs clippy
  # (as a hard error), the test suite, and a format check on top. task-0209.
  checks = {
    # The shipped package itself must build (same derivation as
    # packages.default, so cached) — makes `nix flake check` self-contained
    # rather than relying on nix's packages-building behaviour. task-0209.
    build = package;

    clippy = craneLib.cargoClippy (commonArgs // {
      inherit cargoArtifacts;
      # `-D warnings` mirrors the `just lint` gate: any lint is a build failure.
      cargoClippyExtraArgs = "--all-targets -- --deny warnings";
    });

    test = craneLib.cargoTest (commonArgs // {
      inherit cargoArtifacts;
    });

    fmt = craneLib.cargoFmt {
      inherit (commonArgs) pname version;
      inherit src;
    };
  };
}
