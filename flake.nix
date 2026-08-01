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
        rust = pkgs.rust-bin.stable."1.97.1".default.override {
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

        # Modern-system smoke test (tests/virt-smoke): a stock riscv64 kernel
        # with virtio-blk/ext4 built in, and OpenSBI fw_dynamic, both booted by
        # the virt machine. Exposed as packages so the harness resolves them
        # reproducibly without hard-coded store paths.
        # The distro kernel ships these as modules, which is too late when the
        # root filesystem itself is an ext4 virtio-blk disk. Keep the kernel
        # otherwise stock, but make the boot-critical disk/NIC path built-in.
        # This image does not ship the kernel's module tree into the guest, so
        # packet sockets (used by DHCP clients) must be built in as well.
        virtKernel = pkgs.pkgsCross.riscv64.linux_latest.override {
          structuredExtraConfig = with pkgs.lib.kernel; {
            VIRTIO = yes;
            VIRTIO_MMIO = yes;
            VIRTIO_BLK = yes;
            VIRTIO_NET = yes;
            EXT4_FS = yes;
            PACKET = yes;
            # The proxy exposes its ephemeral public CA before networking via
            # a fixed virtio-9p mount tag. This guest has no module tree, so the
            # complete mount path must be available in the kernel itself.
            NET_9P = yes;
            NET_9P_VIRTIO = yes;
            "9P_FS" = yes;
          };
          ignoreConfigErrors = true;
        };
        # A single-hart kernel for the hardware rv64.js actually implements.
        # Keep the Debian demo's disk, network and 9p paths built in, but remove
        # broad distro hardware support and expensive debug/tracing startup.
        # The stock-derived virtKernel remains the conformance oracle.
        virtKernelFast = pkgs.pkgsCross.riscv64.linux_latest.override {
          structuredExtraConfig = with pkgs.lib.kernel; {
            VIRTIO = yes;
            VIRTIO_MMIO = yes;
            VIRTIO_BLK = yes;
            VIRTIO_NET = yes;
            EXT4_FS = yes;
            PACKET = yes;
            NET_9P = yes;
            NET_9P_VIRTIO = yes;
            "9P_FS" = yes;

            SMP = pkgs.lib.mkForce no;
            NUMA = pkgs.lib.mkForce no;
            EFI = pkgs.lib.mkForce no;
            ACPI = pkgs.lib.mkForce no;
            IPV6 = pkgs.lib.mkForce no;
            CMA = pkgs.lib.mkForce no;
            HUGETLBFS = pkgs.lib.mkForce no;
            HUGETLB_PAGE = pkgs.lib.mkForce no;
            AUDIT = pkgs.lib.mkForce no;
            PERF_EVENTS = pkgs.lib.mkForce no;
            FTRACE = pkgs.lib.mkForce no;
            FUNCTION_TRACER = pkgs.lib.mkForce no;
            DEBUG_KERNEL = pkgs.lib.mkForce no;
            DEBUG_VM_PGTABLE = pkgs.lib.mkForce no;
            KFENCE = pkgs.lib.mkForce no;
            TASKS_RUDE_RCU = pkgs.lib.mkForce no;
            TASKS_TRACE_RCU = pkgs.lib.mkForce no;
          };
          ignoreConfigErrors = true;
        };
        virtOpensbi = pkgs.pkgsCross.riscv64.opensbi;
      in
      {
        packages.virt-kernel = virtKernel;
        packages.virt-kernel-fast = virtKernelFast;
        packages.virt-opensbi = virtOpensbi;

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

            # Debian rootfs bring-up (build-essential in the guest)
            debootstrap # build a riscv64 Debian rootfs (--foreign)
            fakeroot # run debootstrap without real root
            dpkg # dpkg-deb -x for offline .deb extraction
            gnutar
            gzip
            gnused
            wget
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
