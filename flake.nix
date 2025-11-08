{
  description = "Yet Another Mid Ahh Filehost";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/release-25.05";

    crane = {
      url = "github:ipetkov/crane";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    inputs@{
      self,
      nixpkgs,
      crane,
      fenix,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ fenix.overlays.default ];
        };

        inherit (pkgs) lib;

        toolchain = pkgs.fenix.fromToolchainFile {
          file = ./rust-toolchain;
          sha256 = "sha256-NOqZPlm+Fv91JUjZlh3WdjjiaJgmMyhcQGh2SHAp2pM=";
        };

        craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;
        src = ./.;

        commonArgs = { inherit src; };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        yamaf = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
            doCheck = false;
          }
        );
      in
      {
        packages = {
          inherit yamaf;
          default = yamaf;
          image = pkgs.dockerTools.buildImage {
            name = "yamaf";
            config = {
              Cmd = [ "${yamaf}/bin/yamaf" ];
              Env = [ "INTERNAL_HOST=0.0.0.0" ];
            };
          };

          # not using flake checks to run them individually
          checks = {
            clippy = craneLib.cargoClippy (
              commonArgs
              // {
                inherit cargoArtifacts;
              }
            );

            fmt = craneLib.cargoFmt {
              inherit src;
            };
          };
        };

        devShells.default = pkgs.mkShell {
          nativeBuildInputs = [ toolchain ];
        };

        formatter = pkgs.nixfmt-tree;
      }
    );
}
