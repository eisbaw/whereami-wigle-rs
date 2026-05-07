{
  description = "whereami — Wi-Fi geolocation daemon";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      supportedSystems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
      pkgsFor = system: nixpkgs.legacyPackages.${system};
    in {
      packages = forAllSystems (system:
        let pkgs = pkgsFor system; in {
          default = pkgs.callPackage ./nix/package.nix {};
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
