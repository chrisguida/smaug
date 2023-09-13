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
  }: utils.lib.eachDefaultSystem (system:
    let
      cln-overlay = final: prev: {
        cln = prev.clightning.overrideAttrs {
          version = "23.03.2";
          src = prev.fetchFromGitHub {
            owner = "niftynei";
            repo = "lightning";
            rev = "44c5b523683160e8c20bda200c6a5a59ea40bc5e";
            sha256 = "sha256-tWxnuVHhXl7JWwMxQ46b+Jd7PeoMVr7pnWXv5Of5AeI=";
            fetchSubmodules = true;
          };
        };
      };

      pkgs = import nixpkgs {
        inherit system;
        overlays = [ cln-overlay ];
      };
      naersk-lib = pkgs.callPackage naersk {};
    in rec {
      defaultPackage = naersk-lib.buildPackage {
        src = ./.;
        buildInputs = with pkgs; [ libsodium pkg-config openssl cln ];
      };

      devShell = pkgs.mkShell {
          buildInputs = with pkgs; [ cargo rustc rustfmt pre-commit rustPackages.clippy libsodium pkg-config openssl cln ];
          RUST_SRC_PATH = pkgs.rustPlatform.rustLibSrc;
        };
    });
}
