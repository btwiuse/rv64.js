#!/usr/bin/env bash

set -euo pipefail
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/common.sh"

require_source QEMU_WASM_ZLIB_SOURCE
require_source QEMU_WASM_LIBFFI_SOURCE
require_source QEMU_WASM_PIXMAN_SOURCE
require_source QEMU_WASM_GLIB_SOURCE
require_source QEMU_WASM_PCRE2_SOURCE
require_source QEMU_WASM_PCRE2_PATCH

if [[ ! -f "$DEPS_DIR/.zlib-1.3.1" ]]; then
    echo "==> Building zlib 1.3.1 for wasm64"
    extract_once "$QEMU_WASM_ZLIB_SOURCE" "$WORK_DIR/zlib-1.3.1"
    (
        cd "$WORK_DIR/zlib-1.3.1"
        emconfigure ./configure --prefix="$DEPS_DIR" --static
        emmake make -j"$JOBS"
        emmake make install
    )
    touch "$DEPS_DIR/.zlib-1.3.1"
fi

if [[ ! -f "$DEPS_DIR/.libffi-3.5.2" ]]; then
    echo "==> Building libffi 3.5.2 for wasm64"
    extract_once "$QEMU_WASM_LIBFFI_SOURCE" "$WORK_DIR/libffi-3.5.2"
    (
        cd "$WORK_DIR/libffi-3.5.2"
        emconfigure ./configure \
            --host=wasm64-unknown-linux \
            --prefix="$DEPS_DIR" \
            --enable-static \
            --disable-shared \
            --disable-dependency-tracking \
            --disable-builddir \
            --disable-multi-os-directory \
            --disable-raw-api \
            --disable-docs
        emmake make -j"$JOBS"
        emmake make install SUBDIRS=include
    )
    touch "$DEPS_DIR/.libffi-3.5.2"
fi

if [[ ! -f "$DEPS_DIR/.pixman-0.44.2" ]]; then
    echo "==> Building Pixman 0.44.2 for wasm64"
    extract_once "$QEMU_WASM_PIXMAN_SOURCE" "$WORK_DIR/pixman-0.44.2"
    meson_setup "$WORK_DIR/pixman-build" "$WORK_DIR/pixman-0.44.2" \
        --prefix="$DEPS_DIR" \
        --cross-file="$CROSS_FILE" \
        --default-library=static \
        --buildtype=release \
        -Dtests=disabled \
        -Ddemos=disabled
    meson compile -C "$WORK_DIR/pixman-build" -j "$JOBS"
    meson install -C "$WORK_DIR/pixman-build"
    touch "$DEPS_DIR/.pixman-0.44.2"
fi

if [[ ! -f "$DEPS_DIR/lib/libresolv.a" ]]; then
    echo "==> Building Emscripten libresolv compatibility stub"
    emcc "${EM_CFLAGS[@]}" -c "$ROOT_DIR/support/res_query.c" -fPIC \
        -o "$WORK_DIR/res_query.o"
    emar rcs "$DEPS_DIR/lib/libresolv.a" "$WORK_DIR/res_query.o"
fi

if [[ ! -f "$DEPS_DIR/.glib-2.84.0" ]]; then
    echo "==> Building GLib 2.84.0 for wasm64"
    extract_once "$QEMU_WASM_GLIB_SOURCE" "$WORK_DIR/glib-2.84.0"
    mkdir -p "$WORK_DIR/glib-2.84.0/subprojects/packagecache"
    ln -sfn "$QEMU_WASM_PCRE2_SOURCE" \
        "$WORK_DIR/glib-2.84.0/subprojects/packagecache/pcre2-10.44.tar.bz2"
    ln -sfn "$QEMU_WASM_PCRE2_PATCH" \
        "$WORK_DIR/glib-2.84.0/subprojects/packagecache/pcre2_10.44-2_patch.zip"

    GLIB_CFLAGS="$CFLAGS -Wno-incompatible-function-pointer-types" \
        meson_setup "$WORK_DIR/glib-build" "$WORK_DIR/glib-2.84.0" \
            --prefix="$DEPS_DIR" \
            --cross-file="$CROSS_FILE" \
            --default-library=static \
            --buildtype=release \
            --force-fallback-for=pcre2 \
            --wrap-mode=nodownload \
            -Dselinux=disabled \
            -Dxattr=false \
            -Dlibmount=disabled \
            -Dnls=disabled \
            -Dtests=false \
            -Dglib_debug=disabled \
            -Dglib_assert=false \
            -Dglib_checks=false

    sed -i -E '/#define HAVE_POSIX_SPAWN 1/d' "$WORK_DIR/glib-build/config.h"
    sed -i -E '/#define HAVE_PTHREAD_GETNAME_NP 1/d' "$WORK_DIR/glib-build/config.h"
    meson compile -C "$WORK_DIR/glib-build" -j "$JOBS"
    meson install -C "$WORK_DIR/glib-build"
    touch "$DEPS_DIR/.glib-2.84.0"
fi

echo "==> wasm64 dependencies are ready in $DEPS_DIR"
