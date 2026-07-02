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

        # ── Windows cross-compilation ───────────────────────────────────────
        # Minimal stable toolchain with the MinGW Windows target added, plus the
        # MinGW cross-compiler from nixpkgs. Used by `packages.check-windows`
        # (and `checks.check-windows`) to compile-check the workspace for
        # x86_64-pc-windows-gnu from Linux — catching cfg(windows)/type/API
        # breakage without a real Windows runner. libdeflate (the default
        # backend) is built from source by `libdeflate-sys`, so the MinGW gcc
        # must also be the C compiler the `cc` crate uses for that target.
        rust-windows = pkgs.rust-bin.stable.latest.minimal.override {
          targets = [ "x86_64-pc-windows-gnu" ];
        };
        mingw = pkgs.pkgsCross.mingwW64.stdenv.cc;
        naersk-lib-windows = naersk.lib."${system}".override {
          cargo = rust-windows;
          rustc = rust-windows;
        };

        # ── Fully static (musl) Linux binary ────────────────────────────────
        # The truly portable "runs on any Linux distro" build: statically linked
        # against musl libc, so there is no glibc version coupling (the glibc
        # binary above is built against a very recent nixpkgs glibc and would
        # fail the symbol-version check on older distros). libdeflate's C source
        # is compiled for the musl target by the musl cross-gcc and linked in.
        rust-musl = pkgs.rust-bin.stable.latest.default.override {
          targets = [ "x86_64-unknown-linux-musl" ];
        };
        naersk-lib-musl = naersk.lib."${system}".override {
          cargo = rust-musl;
          rustc = rust-musl;
        };
        muslCC = pkgs.pkgsCross.musl64.stdenv.cc;

        bacon = pkgs.bacon;

      in
      rec {
        packages.cargo-testmap =
          (naersk-lib.buildPackage {
            pname = "cargo-testmap";
            root = ./cargo-testmap;
            nativeBuildInputs = with pkgs; [ pkg-config ];
            buildInputs = with pkgs; [ hdf5 ];
            release = true;
            CARGO_PROFILE_RELEASE_debug = "0";
          }).overrideAttrs
            {
              postInstall = ''
                install -Dm644 completions/cargo-testmap.fish \
                  $out/share/fish/vendor_completions.d/cargo-testmap.fish
              '';
            };

        # `nix build .#static-linux` → fully static musl binary. Has no ELF
        # interpreter and no dynamic dependencies at all, so it runs unchanged on
        # any Linux distribution (Alpine, old CentOS, Debian, …) regardless of
        # its libc. CI executes this one inside plain Alpine and Debian
        # containers (ci.yml).
        packages.static-linux = naersk-lib-musl.buildPackage {
          pname = "cargo-rustmap-static";
          root = ./cargo-testmap;
          CARGO_BUILD_TARGET = "x86_64-unknown-linux-musl";
          # libdeflate-sys builds libdeflate's C source; for the musl target it
          # must use the musl cross-gcc so the objects link into the static binary.
          CC_x86_64_unknown_linux_musl = "${muslCC}/bin/${muslCC.targetPrefix}cc";
          nativeBuildInputs = [ muslCC ];
          release = true;
          CARGO_PROFILE_RELEASE_debug = "0";
          postInstall = "rm -f $out/bin/bench-inflate";
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
          src = ./cargo-testmap;
          mode = "check";
          name = "cargo-testmap";
          nativeBuildInputs = with pkgs; [ pkg-config ];
          buildInputs = with pkgs; [ hdf5 ];
        };

        packages.test = naersk-lib.buildPackage {
          pname = "cargo-testmap";
          root = ./cargo-testmap;
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
            pkgs.cargo-deny
            pkgs.lcov
          ];
        };
      }
    );
}
