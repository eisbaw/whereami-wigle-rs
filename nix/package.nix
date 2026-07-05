{ lib
, makeWrapper
, pkg-config
, openssl
, sqlite
, iw
, networkmanager
, systemd
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
  # crane's cleanCargoSource keeps only Cargo files and *.rs — which silently
  # DROPS whereamid/tests/proptests.proptest-regressions, the committed
  # proptest counterexample corpus (task-0071). Without it the `test` check
  # runs proptests WITHOUT the deterministic regression pins that `just test`
  # has. server.rs also compiles the exact docs/protocol.md into a protocol
  # contract test via include_str!, so that single non-Cargo input must be in
  # the source closure too; including all docs would needlessly invalidate the
  # Rust build for unrelated documentation edits.
  src = lib.cleanSourceWith {
    src = ./..;
    filter = path: type:
      (craneLib.filterCargoSources path type)
      || lib.hasSuffix ".proptest-regressions" path
      || lib.hasSuffix "/docs/protocol.md" path;
    name = "source";
  };

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
        --prefix PATH : ${lib.makeBinPath [ iw networkmanager systemd ]}
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
      # Same flags as `just lint` (any lint is a build failure), but NOT the
      # same toolchain or profile: this runs nixpkgs STABLE clippy in crane's
      # default RELEASE profile, while the devShell just-gate runs fenix
      # NIGHTLY in debug. Lint sets differ across channels and debug_assert!/
      # overflow semantics differ across profiles, so the two gates are
      # complementary, not mirrors. task-0235 tracks unify-vs-keep.
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
