{ lib
, rustPlatform
, makeWrapper
, pkg-config
, openssl
, sqlite
, iw
, networkmanager
}:

let
  cargoToml = builtins.fromTOML (builtins.readFile ../whereamid/Cargo.toml);
in
rustPlatform.buildRustPackage {
  pname = "whereamid";
  version = cargoToml.package.version;

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

  meta = with lib; {
    description = "Wi-Fi geolocation daemon";
    license = licenses.mit;
    mainProgram = "whereamid";
  };
}
