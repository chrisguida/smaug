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
      pkgs = import nixpkgs {
        inherit system;
      };
      naersk-lib = pkgs.callPackage naersk {};
    in rec {
      defaultPackage = naersk-lib.buildPackage {
        src = ./.;
        buildInputs = with pkgs; [ pkg-config openssl clightning ];
      };

      devShell = pkgs.mkShell {
        buildInputs = with pkgs; [ bash bitcoin clightning cargo gawk libeatmydata openssl pkg-config poetry pre-commit rustc rustfmt rustPackages.clippy ];
        RUST_SRC_PATH = pkgs.rustPlatform.rustLibSrc;
        shellHook = ''
          echo "Entering devshell..."

          echo "Run \`cargo build\` to build \`smaug\`."
          echo ""
          echo "If this is your first time setting up smaug, run:"
          echo "mkdir -p ~/.bitcoin"
          echo ""

          # Extract CLN zip file to a temporary directory
          TMP_DIR=$(mktemp -d)
          unzip -q ${pkgs.clightning.src} -d "$TMP_DIR"

          echo "Then to set up two lightning nodes and a bitcoin node in regtest mode,"
          echo "run the following two commands:"
          echo "source $TMP_DIR/clightning-v24.08.1/contrib/startup_regtest.sh"
          echo "start_ln"
          echo ""

          echo "Finally, to start smaug on Lightning Node 1, run"
          echo "l1-cli plugin start $(pwd)/target/debug/smaug"
        '';
      };
    }
  );
}
