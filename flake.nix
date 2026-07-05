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
      lib = nixpkgs.lib;
      supportedSystems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = lib.genAttrs supportedSystems;
      pkgsFor = system: nixpkgs.legacyPackages.${system};

      # The flake's own git revision, resolved PURELY (no fetchGit) so
      # `nix flake check` can evaluate. Clean tree -> shortRev; dirty working
      # tree (e.g. mid-development) -> dirtyShortRev; detached/unknown ->
      # "unknown". Threaded into package.nix's preBuild. task-0209.
      gitRev = self.shortRev or self.dirtyShortRev or "unknown";

      # Single entry point for the crane-built package + its checks, so the
      # dependency artifacts are computed once and shared (package.nix).
      whereamiFor = system:
        let pkgs = pkgsFor system; in
        pkgs.callPackage ./nix/package.nix {
          craneLib = crane.mkLib pkgs;
          inherit gitRev;
        };
    in {
      packages = forAllSystems (system: {
        default = (whereamiFor system).package;
      });

      # `nix flake check` runs these for every supported system: the package
      # build, clippy (-D warnings), the test suite and a format check, plus a
      # NixOS VM test that boots the module (x86_64-linux only — needs KVM).
      # They share the crane cargoArtifacts (built once per system). task-0209.
      checks = forAllSystems (system:
        (whereamiFor system).checks
        // {
          # Force the shared module option's negative upper-bound evaluation on
          # every supported system; unlike flag parity this does not execute a
          # target binary and is therefore architecture-independent.
          option-bounds = import ./nix/option-bounds.nix {
            pkgs = pkgsFor system;
            inherit lib;
          };
        }
        // lib.optionalAttrs (system == "x86_64-linux") {
          vm = import ./nix/vm-test.nix {
            inherit self gitRev;
            pkgs = pkgsFor system;
          };
          # Guards against clap-flag / module-option drift (task-0206). Execs the
          # built binary's --help, so host-arch only (like the VM check).
          flag-parity = import ./nix/flag-parity.nix {
            pkgs = pkgsFor system;
            inherit lib;
            package = (whereamiFor system).package;
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
              pkgs.just
              pkgs.pkg-config
              pkgs.openssl
              pkgs.sqlite
              pkgs.iw
              pkgs.networkmanager
            ];

            OPENSSL_DEV = pkgs.openssl.dev;
            PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";

            # Make runtime libraries discoverable for binaries that the
            # build tools link without baking an rpath:
            #   - libstdc++ for cargo-fuzz sanitizer-instrumented binaries
            #   - libssl/libcrypto for cargo-test binaries that load reqwest
            shellHook = ''
              export LD_LIBRARY_PATH="${pkgs.stdenv.cc.cc.lib}/lib:${pkgs.openssl.out}/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
            '';
          };
        }
      );

      nixosModules.default = { pkgs, lib, ... }: {
        imports = [ ./nix/module.nix ];
        services.whereami.package = lib.mkDefault (pkgs.callPackage ./nix/package.nix {
          craneLib = crane.mkLib pkgs;
          inherit gitRev;
        }).package;
      };

      homeManagerModules.default = { pkgs, lib, ... }: {
        imports = [ ./nix/hm-module.nix ];
        services.whereami.package = lib.mkDefault (pkgs.callPackage ./nix/package.nix {
          craneLib = crane.mkLib pkgs;
          inherit gitRev;
        }).package;
      };
    };
}
