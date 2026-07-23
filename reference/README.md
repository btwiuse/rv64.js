# reference/

## tinyemu/

TinyEMU 2019-12-21 by Fabrice Bellard, vendored verbatim from
<https://bellard.org/tinyemu/tinyemu-2019-12-21.tar.gz>.

License: MIT (see `tinyemu/MIT-LICENSE.txt`). Note the x86 machine inside it
mentions its own licensing in `readme.txt`; we only use the RISC-V side.

Role in this project:

1. **Spec map** — `riscv_cpu.c`, `riscv_machine.c`, `virtio.c` define the
   exact subset of RISC-V + devices sufficient to boot mainline Linux.
2. **Oracle** — built natively (`make CONFIG_FS_NET= CONFIG_SDL=
   CONFIG_X86EMU= CONFIG_SLIRP=`) for differential testing against rv64.js.

We port scope and ideas, not code. Do **not** copy from the jslinux.org
website bundle — unlike this C source, it is not open source.
