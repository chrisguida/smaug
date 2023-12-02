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
        clightning = prev.clightning.overrideAttrs {
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
        buildInputs = with pkgs; [ pkg-config openssl clightning ];
      };

      devShell = pkgs.mkShell {
        buildInputs = with pkgs; [ bash cargo rustc rustfmt pre-commit rustPackages.clippy pkg-config openssl bitcoin clightning ];
        RUST_SRC_PATH = pkgs.rustPlatform.rustLibSrc;
        shellHook = ''
          echo "Entering devshell..."

          echo "Run \`cargo build\` to build \`smaug\`."
          echo ""
          echo "If this is your first time setting up smaug, run:"
          echo "mkdir -p ~/.bitcoin"
          echo ""

          echo "Then to set up two lightning nodes and a bitcoin node in regtest mode,"
          echo "run the following two commands:"
          echo "source ${pkgs.clightning.src}/contrib/startup_regtest.sh"
          echo "start_ln"
          echo ""

          echo "Finally, to start smaug on Lightning Node 1, run"
          echo "l1-cli plugin start $(pwd)/target/debug/smaug"
        '';
      };
    }
  );
}
