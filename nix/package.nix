{ lib
, makeWrapper
, pkg-config
, openssl
, sqlite
, iw
, networkmanager
, craneLib
}:

let
  # Get the source with git rev info preserved
  gitSrc = builtins.fetchGit {
    url = ./..;
    allRefs = true;
  };
  gitRev = gitSrc.shortRev or "unknown";

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

  # Phase 1: deps only (cached until Cargo.lock changes)
  cargoArtifacts = craneLib.buildDepsOnly commonArgs;

in
  # Phase 2: workspace code (reuses cached deps)
  craneLib.buildPackage (commonArgs // {
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
  })
