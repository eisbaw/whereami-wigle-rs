{ lib
, rustPlatform
, makeWrapper
, pkg-config
, openssl
, sqlite
, iw
, networkmanager
, gitRev ? "unknown"
}:

rustPlatform.buildRustPackage {
  pname = "whereamid";
  version = "0.2.1";

  src = lib.cleanSource ./..;

  cargoLock.lockFile = ../Cargo.lock;

  nativeBuildInputs = [
    makeWrapper
    pkg-config
  ];

  buildInputs = [
    openssl
    sqlite
  ];

  # Write git rev file for build.rs to pick up (no .git in nix sandbox)
  preBuild = ''
    echo "${gitRev}" > whereamid/GIT_REV
    echo "${gitRev}" > whereami-client/GIT_REV
  '';

  postInstall = ''
    wrapProgram $out/bin/whereamid \
      --prefix PATH : ${lib.makeBinPath [ iw networkmanager ]}
  '';

  meta = with lib; {
    description = "Wi-Fi geolocation daemon for NixOS";
    license = licenses.mit;
    mainProgram = "whereamid";
  };
}
