{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/release-26.05";
    utils.url = "github:numtide/flake-utils";
    naersk.url = "github:nmattia/naersk";
    naersk.inputs.nixpkgs.follows = "nixpkgs";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      self,
      nixpkgs,
      utils,
      naersk,
      rust-overlay,
    }:
    utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
        rust = pkgs.rust-bin.stable."1.93.1".default.override {
          targets = [ "x86_64-unknown-linux-musl" ];
          extensions = [
            "llvm-tools-preview"
            "rust-analyzer"
          ];
        };

        naersk-lib = naersk.lib."${system}".override {
          cargo = rust;
          rustc = rust;
        };

      in
      rec {
        packages.cargo-testmap =
          (naersk-lib.buildPackage {
            pname = "cargo-testmap";
            root = ./cargo_testmap;
            nativeBuildInputs = with pkgs; [ pkg-config ];
            buildInputs = with pkgs; [ hdf5 ];
            release = true;
            CARGO_PROFILE_RELEASE_debug = "0";
          }).overrideAttrs
            {
              postInstall = ''
                install -Dm644 ${./cargo_testmap/completions/cargo-testmap.fish} \
                  $out/share/fish/vendor_completions.d/cargo-testmap.fish
              '';
            };

        packages.cargo-testmap_other_linux =
          (naersk-lib.buildPackage {
            pname = "cargo-testmap";
            root = ./cargo_testmap;
            nativeBuildInputs = with pkgs; [
              pkg-config
              patchelf
            ];
            buildInputs = with pkgs; [ hdf5 ];
            release = true;
            CARGO_PROFILE_RELEASE_debug = "0";
          }).overrideAttrs
            {
              postInstall = ''
                patchelf $out/bin/cargo-testmap --set-interpreter "/lib64/ld-linux-x86-64.so.2"
              '';
            };

        packages.cargo-testmap-docker =
          let
            binary = packages.cargo-testmap_other_linux;
          in
          pkgs.dockerTools.buildLayeredImage {
            name = "cargo-testmap";
            tag = "latest";
            contents = [
              pkgs.busybox
              pkgs.glibc
              pkgs.hdf5
              binary
            ];
            config = {
              Env = [ "PATH=/usr/local/bin:/bin" ];
              Entrypoint = [ "/bin/cargo-testmap" ];
              WorkingDir = "/work";
            };
          };

        packages.check = naersk-lib.buildPackage {
          src = ./cargo_testmap;
          mode = "check";
          name = "cargo-testmap";
          nativeBuildInputs = with pkgs; [ pkg-config ];
          buildInputs = with pkgs; [ hdf5 ];
        };

        packages.test = naersk-lib.buildPackage {
          pname = "cargo-testmap";
          root = ./cargo_testmap;
          mode = "test";
          nativeBuildInputs = with pkgs; [ pkg-config ];
          buildInputs = with pkgs; [ hdf5 ];
        };

        defaultPackage = packages.cargo-testmap;

        apps.cargo-testmap = utils.lib.mkApp { drv = packages.cargo-testmap; };
        defaultApp = apps.cargo-testmap;

        devShell = pkgs.mkShell {
          shellHook = ''
            #export RUSTFLAGS="-C link-arg=-fuse-ld=mold"
          '';
          nativeBuildInputs = [
            pkgs.bacon
            pkgs.cargo-nextest
            pkgs.mold
            pkgs.pkg-config
            pkgs.ripgrep
            rust
            pkgs.cargo-llvm-cov
            pkgs.cargo-llvm-lines
            pkgs.lcov
          ];
        };
      }
    );
}
