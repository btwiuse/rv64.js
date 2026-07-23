{
  description = "rv64.js — RISC-V emulator in Rust/wasm that boots Linux in the browser";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.05";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        # Rust with the two cross targets the project needs:
        # - wasm32-unknown-unknown: the browser build (rv64-wasm)
        # - riscv64gc-unknown-linux-musl: guest test binaries (guests/*)
        rust = pkgs.rust-bin.stable.latest.default.override {
          targets = [
            "wasm32-unknown-unknown"
            "riscv64gc-unknown-linux-musl"
          ];
        };

        # Bare-metal RISC-V cross compiler for building the official
        # riscv-tests ISA suite (tests/run-isa-tests.sh).
        riscvGcc = pkgs.pkgsCross.riscv64-embedded.buildPackages.gcc;

        # Spike with commit logging enabled (tests/lockstep.py needs
        # --log-commits, which is a compile-time option).
        spike = pkgs.spike.overrideAttrs (old: {
          configureFlags = (old.configureFlags or [ ]) ++ [ "--enable-commitlog" ];
        });
      in
      {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            rust

            # native builds (TinyEMU oracle, Spike) + scripts
            gcc
            gnumake
            autoconf
            automake
            python3
            curl
            git

            # JS harness for the wasm build (web/rv64.js, smoke tests)
            nodejs_20

            # validation oracles
            qemu # qemu-riscv64 (user) + qemu-system-riscv64
            spike # riscv-isa-sim golden model (commit logging enabled above)
            dtc # device-tree-compiler (Spike runtime dependency)

            # wasm tooling: validate/disassemble JIT-emitted modules
            wabt # wasm-validate, wasm2wat
            binaryen # wasm-opt

            # riscv-tests cross build
            riscvGcc

            # modern-system bring-up (virt machine): OpenSBI + kernel + rootfs
            cpio # initramfs packing
            e2fsprogs # mke2fs/debugfs/resize2fs — guest disk images
            util-linux # sfdisk/losetup helpers
            zstd # image (de)compression
          ];

          shellHook = ''
            # riscv64-embedded cross gcc uses the riscv64-none-elf- prefix;
            # tests/run-isa-tests.sh honors RISCV_PREFIX.
            export RISCV_PREFIX=riscv64-none-elf-
            echo "rv64.js dev shell — run tests/run-all.sh for the full suite"
          '';
        };
      });
}
