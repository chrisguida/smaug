{
  inputs = {
    naersk.url = "github:nix-community/naersk/master";
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    utils,
    naersk,
    ...
  }:
  let
      cln-onchain-notif = final: prev: {
        clightning = prev.clightning.overrideAttrs (old: {
          src = prev.fetchFromGitHub {
            owner = "niftynei";
            repo = "lightning";
            rev = "44c5b523683160e8c20bda200c6a5a59ea40bc5e";
            sha256 = "sha256-9T5d5W12v4b88FmCRiXhgQrzXOSCR5ByzT1ZJ05FI3A=";
            fetchSubmodules = true;
          };
        });
      };
      pkgsForSystem = system: import nixpkgs {
        inherit system;
        overlays = [cln-onchain-notif];
      };
    in utils.lib.eachDefaultSystem (system: rec {
      legacyPackages = pkgsForSystem system;
      naersk-lib = legacyPackages.callPackage naersk {};
      packageDefault = naersk-lib.buildPackage ./.;
      devShells.default = with legacyPackages; mkShell {
          buildInputs = [cargo rustc rustfmt pre-commit rustPackages.clippy clightning libsodium pkg-config ];
          RUST_SRC_PATH = rustPlatform.rustLibSrc;
        };
    });
}
