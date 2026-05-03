{ lib
, rustPlatform
, makeWrapper
, pkg-config
, openssl
, sqlite
, iw
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
      --prefix PATH : ${lib.makeBinPath [ iw ]}
  '';

  meta = with lib; {
    description = "Wi-Fi geolocation daemon for NixOS";
    license = licenses.mit;
    mainProgram = "whereamid";
  };
}
