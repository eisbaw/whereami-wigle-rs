{
  description = "whereami — Wi-Fi geolocation daemon";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, crane }:
    let
      supportedSystems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
      pkgsFor = system: nixpkgs.legacyPackages.${system};
    in {
      packages = forAllSystems (system:
        let
          pkgs = pkgsFor system;
          craneLib = crane.mkLib pkgs;
          gitRev = self.shortRev or self.dirtyShortRev or "unknown";

          commonArgs = {
            pname = "whereami";
            version = "0.2.1";
            src = craneLib.cleanCargoSource ./.;
            strictDeps = true;

            nativeBuildInputs = with pkgs; [
              pkg-config
              makeWrapper
            ];

            buildInputs = with pkgs; [
              openssl
              sqlite
            ];

            # Write git rev for build.rs (no .git in sandbox)
            preBuild = ''
              echo "${gitRev}" > whereamid/GIT_REV
              echo "${gitRev}" > whereami-client/GIT_REV
            '';
          };

          # Phase 1: build only dependencies (cached until Cargo.lock changes)
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;

          # Phase 2: build the workspace (reuses cached deps)
          whereami = craneLib.buildPackage (commonArgs // {
            inherit cargoArtifacts;

            postInstall = ''
              wrapProgram $out/bin/whereamid \
                --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.iw pkgs.networkmanager ]}
            '';

            meta = with pkgs.lib; {
              description = "Wi-Fi geolocation daemon for NixOS";
              license = licenses.mit;
              mainProgram = "whereamid";
            };
          });
        in {
          default = whereami;
          whereamid = whereami;
        }
      );

      devShells = forAllSystems (system:
        let pkgs = pkgsFor system; in {
          default = pkgs.mkShell {
            buildInputs = with pkgs; [
              rustc
              cargo
              clippy
              rustfmt
              pkg-config
              openssl
              sqlite
              iw
              networkmanager
            ];

            OPENSSL_DEV = pkgs.openssl.dev;
            PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
          };
        }
      );

      nixosModules.default = import ./nix/module.nix;
    };
}
