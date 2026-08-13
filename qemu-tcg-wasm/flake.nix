{
  description = "Reproducible build environment for QEMU's proposed wasm64 TCG backend";

  inputs.nixpkgs.url = "tarball+https://codeload.github.com/NixOS/nixpkgs/tar.gz/42f2e0330f72a8f3593586f0acd57f2620bb1ee6";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          python = pkgs.python3.withPackages (ps: [ ps.tomli ]);

          zlibSource = pkgs.fetchurl {
            url = "https://zlib.net/fossils/zlib-1.3.1.tar.gz";
            hash = "sha256-mpOyt9/ax3zrpaVYpYDnRmfdb+3kWFuR7vtg8Dty3yM=";
          };
          libffiSource = pkgs.fetchurl {
            url = "https://github.com/libffi/libffi/releases/download/v3.5.2/libffi-3.5.2.tar.gz";
            hash = "sha256-86MIKiOzfCk6T80QUxR7Nx8v+R+n6hsqUuM1Z2usgtw=";
          };
          pixmanSource = pkgs.fetchurl {
            url = "https://cairographics.org/releases/pixman-0.44.2.tar.gz";
            hash = "sha256-Y0kGHOGjOKtpUrkhlNGwN3RyJEII1H/yW++G/HGXNGY=";
          };
          glibSource = pkgs.fetchurl {
            url = "https://download.gnome.org/sources/glib/2.84/glib-2.84.0.tar.xz";
            hash = "sha256-+II2AMuFQl4oFc+tguog/apThIKrdOcpPViz9kpa/2o=";
          };
          pcre2Source = pkgs.fetchurl {
            url = "https://github.com/PCRE2Project/pcre2/releases/download/pcre2-10.44/pcre2-10.44.tar.bz2";
            hash = "sha256-008C4RPPcZOh6/J3DTrFJwiNSF1OBH7RDl0hfG713pY=";
          };
          pcre2Patch = pkgs.fetchurl {
            url = "https://wrapdb.mesonbuild.com/v2/pcre2_10.44-2/get_patch";
            name = "pcre2_10.44-2_patch.zip";
            hash = "sha256-QzbUIu6QQ4R+XhDbu9AZQNTJ5QJ/MczcM6eJihypQAk=";
          };
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              autoconf
              automake
              bison
              binaryen
              emscripten
              file
              flex
              gcc
              git
              glib.bin
              glib.dev
              gnumake
              jq
              libtool
              meson
              ninja
              nodejs_24
              pkg-config
              python
              shellcheck
              wabt
              which
            ];

            shellHook = ''
              export QEMU_WASM_ZLIB_SOURCE="${zlibSource}"
              export QEMU_WASM_LIBFFI_SOURCE="${libffiSource}"
              export QEMU_WASM_PIXMAN_SOURCE="${pixmanSource}"
              export QEMU_WASM_GLIB_SOURCE="${glibSource}"
              export QEMU_WASM_PCRE2_SOURCE="${pcre2Source}"
              export QEMU_WASM_PCRE2_PATCH="${pcre2Patch}"
              export EM_CACHE="''${EM_CACHE:-$PWD/.cache/emscripten}"
              mkdir -p "$EM_CACHE"
            '';
          };
        }
      );

      formatter = forAllSystems (system: nixpkgs.legacyPackages.${system}.nixfmt);
    };
}
