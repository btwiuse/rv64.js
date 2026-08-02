#!/bin/sh
# Fetch the TinyEMU riscv64 Linux guest images (Fabrice Bellard, bellard.org)
# into web/images/ for the browser demo.
set -e
cd "$(dirname "$0")"
mkdir -p images
cd images
if [ ! -f bbl64.bin ]; then
    curl -sL -o diskimage.tar.gz https://bellard.org/tinyemu/diskimage-linux-riscv-2018-09-23.tar.gz
    echo "808ecc1b32efdd76103172129b77b46002a616dff2270664207c291e4fde9e14  diskimage.tar.gz" \
        | sha256sum -c -
    tar xzf diskimage.tar.gz --strip-components=1 \
        diskimage-linux-riscv-2018-09-23/bbl64.bin \
        diskimage-linux-riscv-2018-09-23/kernel-riscv64.bin \
        diskimage-linux-riscv-2018-09-23/root-riscv64.bin
    rm diskimage.tar.gz
fi
echo "images ready: $(ls -la bbl64.bin kernel-riscv64.bin root-riscv64.bin | wc -l) files"
