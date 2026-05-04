{ lib
, rustPlatform
, makeWrapper
, pkg-config
, openssl
, sqlite
, iw
, networkmanager
}:

rustPlatform.buildRustPackage {
  pname = "whereamid";
  version = "0.1.0";

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

  postInstall = ''
    wrapProgram $out/bin/whereamid \
      --prefix PATH : ${lib.makeBinPath [ iw networkmanager ]}
  '';

  # Both whereamid (daemon) and whereami (CLI) are built by the workspace

  meta = with lib; {
    description = "Wi-Fi geolocation daemon for NixOS";
    license = licenses.mit;
    mainProgram = "whereamid";
  };
}
