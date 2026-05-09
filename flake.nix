{
  description = "whereami — Wi-Fi geolocation daemon";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, crane, fenix }:
    let
      supportedSystems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
      pkgsFor = system: nixpkgs.legacyPackages.${system};
    in {
      packages = forAllSystems (system:
        let pkgs = pkgsFor system; in {
          default = pkgs.callPackage ./nix/package.nix {
            craneLib = crane.mkLib pkgs;
          };
        }
      );

      devShells = forAllSystems (system:
        let
          pkgs = pkgsFor system;
          fenixPkgs = fenix.packages.${system};
          # Nightly toolchain for cargo-fuzz
          nightlyToolchain = fenixPkgs.combine [
            fenixPkgs.latest.cargo
            fenixPkgs.latest.rustc
            fenixPkgs.latest.clippy
            fenixPkgs.latest.rustfmt
          ];
        in {
          default = pkgs.mkShell {
            buildInputs = [
              nightlyToolchain
              pkgs.cargo-fuzz
              pkgs.pkg-config
              pkgs.openssl
              pkgs.sqlite
              pkgs.iw
              pkgs.networkmanager
            ];

            OPENSSL_DEV = pkgs.openssl.dev;
            PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
          };
        }
      );

      nixosModules.default = { pkgs, lib, ... }: {
        imports = [ ./nix/module.nix ];
        services.whereami.package = lib.mkDefault (pkgs.callPackage ./nix/package.nix {
          craneLib = crane.mkLib pkgs;
        });
      };
    };
}
