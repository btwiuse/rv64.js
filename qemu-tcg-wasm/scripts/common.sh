#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"
QEMU_DIR="$ROOT_DIR/qemu"
WORK_DIR="$ROOT_DIR/.work"
DEPS_DIR="$ROOT_DIR/.deps/wasm64"
CROSS_FILE="$WORK_DIR/cross.meson"
JOBS="${JOBS:-$(nproc)}"

if [[ -z "${IN_NIX_SHELL:-}" ]]; then
    echo "error: enter the flake first with: nix develop path:." >&2
    exit 1
fi

for required_tool in \
    emcc emconfigure emmake em-config file meson ninja node pkg-config python \
    timeout wasm-objdump; do
    if ! command -v "$required_tool" >/dev/null 2>&1; then
        echo "error: missing $required_tool; run this from nix develop path:." >&2
        exit 1
    fi
done

mkdir -p "$WORK_DIR" "$DEPS_DIR" "$ROOT_DIR/.cache/emscripten"
sed "s|@DEPS_DIR@|$DEPS_DIR|g" "$ROOT_DIR/cross.meson.in" > "$CROSS_FILE"

export EM_CACHE="${EM_CACHE:-$ROOT_DIR/.cache/emscripten}"
export CPATH="$DEPS_DIR/include"
export PKG_CONFIG_PATH="$DEPS_DIR/lib/pkgconfig"
export PKG_CONFIG_LIBDIR="$DEPS_DIR/lib/pkgconfig"
export EM_PKG_CONFIG_PATH="$DEPS_DIR/lib/pkgconfig"
export PKG_CONFIG_ALLOW_CROSS=1
EM_CFLAGS=(-O3 -pthread -DWASM_BIGINT -sMEMORY64=1)
export CFLAGS="${EM_CFLAGS[*]}"
export CXXFLAGS="$CFLAGS"
export LDFLAGS="-sWASM_BIGINT -sASYNCIFY=1 -sMEMORY64=1 -L$DEPS_DIR/lib"

require_source() {
    local variable_name="$1"
    if [[ -z "${!variable_name:-}" || ! -f "${!variable_name}" ]]; then
        echo "error: $variable_name was not supplied by the Nix flake" >&2
        exit 1
    fi
}

extract_once() {
    local archive="$1"
    local destination="$2"
    if [[ ! -d "$destination" ]]; then
        tar -xf "$archive" -C "$WORK_DIR"
    fi
}

meson_setup() {
    local build_dir="$1"
    shift
    if [[ -d "$build_dir/meson-private" ]]; then
        meson setup --wipe "$build_dir" "$@"
    else
        meson setup "$build_dir" "$@"
    fi
}
