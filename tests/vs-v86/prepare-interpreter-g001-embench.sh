#!/usr/bin/env bash

# Build and seal the untouched Embench-IoT G001 transfer population. This
# script compiles and packages both architectures but deliberately never runs
# a benchmark or boots an emulator.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ARTIFACTS="${1:?usage: prepare-interpreter-g001-embench.sh ARTIFACTS EMBENCH_SOURCE [OUTPUT_DIR]}"
SOURCE="${2:?usage: prepare-interpreter-g001-embench.sh ARTIFACTS EMBENCH_SOURCE [OUTPUT_DIR]}"
ARTIFACTS="$(cd "$ARTIFACTS" && pwd)"
SOURCE="$(cd "$SOURCE" && pwd)"
OUTPUT="${3:-$ARTIFACTS/interpreter-g001-embench-v1}"
mkdir -p "$OUTPUT"
OUTPUT="$(cd "$OUTPUT" && pwd)"
ZIG="${ZIG:-$(command -v zig)}"

UPSTREAM_COMMIT=09c2ed8c3b7008c95d08b038de4a3f6dc103ed70
UPSTREAM_ARCHIVE_SHA256=90d8c3efef1a7c8c19748b465757c79c711990ea1be94753ff5a78c1217d0969
ZIG_VERSION=0.16.0
ZIG_SHA256=190a19fb057d44a1ed3b44bff68a218b29d99fc6dec8dd2353878c0e46f18c91

benchmarks=(
    aha-mont64
    crc32
    depthconv
    edn
    huffbench
    matmult-int
    md5sum
    nettle-aes
    nettle-sha256
    nsichneu
    picojpeg
    qrduino
    sglib-combined
    slre
    statemate
    tarfind
    ud
    wikisort
    xgboost
)

INIT="$HERE/interpreter-g001/embench-init"
CONTRACT="$HERE/interpreter-g001/embench-contract.json"
RV64_BASE="$ARTIFACTS/scorecard-v2-modern-riscv64.cpio"
I386_BASE="$ARTIFACTS/scorecard-v2-modern-x86.cpio"

for required in "$ZIG" "$INIT" "$CONTRACT" "$RV64_BASE" "$I386_BASE"; do
    test -f "$required" || { echo "missing G001 input: $required" >&2; exit 2; }
done
test ! -e "$OUTPUT/SHA256SUMS" || {
    echo "refusing to rebuild already sealed G001 population: $OUTPUT" >&2
    exit 2
}

test "$(git -C "$SOURCE" rev-parse HEAD)" = "$UPSTREAM_COMMIT" || {
    echo "Embench checkout is not at frozen commit $UPSTREAM_COMMIT" >&2
    exit 2
}
test -z "$(git -C "$SOURCE" status --porcelain)" || {
    echo "Embench checkout is not clean" >&2
    exit 2
}
archive_sha="$(git -C "$SOURCE" archive --format=tar HEAD | sha256sum | awk '{print $1}')"
test "$archive_sha" = "$UPSTREAM_ARCHIVE_SHA256" || {
    echo "Embench archive hash mismatch: $archive_sha" >&2
    exit 2
}
test "$("$ZIG" version)" = "$ZIG_VERSION" || {
    echo "Zig version mismatch" >&2
    exit 2
}
printf '%s  %s\n' "$ZIG_SHA256" "$ZIG" | sha256sum -c -
printf '%s  %s\n' \
    cbb75afb016d8965d56dda71024cdc9a5ce068bdd86019f4fee451c0da04b808 \
    "$RV64_BASE" | sha256sum -c -
printf '%s  %s\n' \
    f626064d8ca2a2031f00b3e6389ba2a65866535df0a143ad0b02ab92a7f70be5 \
    "$I386_BASE" | sha256sum -c -

mapfile -t discovered < <(
    find "$SOURCE/src" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' | LC_ALL=C sort
)
test "${#discovered[@]}" -eq "${#benchmarks[@]}" || {
    echo "Embench benchmark count mismatch" >&2
    exit 2
}
for index in "${!benchmarks[@]}"; do
    test "${discovered[$index]}" = "${benchmarks[$index]}" || {
        echo "Embench inventory mismatch at $index" >&2
        exit 2
    }
done

mkdir -p "$OUTPUT/binaries/rv64" "$OUTPUT/binaries/i386"
commands="$OUTPUT/BUILD-COMMANDS.txt"
: > "$commands"
common=(
    -O2
    -static
    -ffunction-sections
    -fdata-sections
    -Wl,--gc-sections
    -DWARMUP_HEAT=0
    -DGLOBAL_SCALE_FACTOR=1
    -DHAVE_BOARDSUPPORT_H=1
    -Isupport
    -Iexamples/native/speed
)

build_arch() {
    local suffix="$1"
    local target="$2"
    local benchmark
    local -a sources command

    for benchmark in "${benchmarks[@]}"; do
        mapfile -d '' -t sources < <(
            find "src/$benchmark" -maxdepth 1 -type f -name '*.c' -print0 | LC_ALL=C sort -z
        )
        command=(
            "$ZIG" cc -target "$target" "${common[@]}"
            -o "$OUTPUT/binaries/$suffix/$benchmark"
            "${sources[@]}"
            support/main.c
            support/beebsc.c
            examples/native/speed/boardsupport.c
            -lm
        )
        printf '%q ' "${command[@]}" >> "$commands"
        printf '\n' >> "$commands"
        "${command[@]}"
    done
}

(
    cd "$SOURCE"
    build_arch rv64 riscv64-linux-musl
    build_arch i386 x86-linux-musl
)

scratch="$(mktemp -d "${TMPDIR:-/tmp}/rv64-g001-embench.XXXXXX")"
trap 'rm -rf -- "$scratch"' EXIT
for suffix in rv64 i386; do
    root="$scratch/root-$suffix"
    mkdir -p "$root"
    if [ "$suffix" = rv64 ]; then
        base="$RV64_BASE"
    else
        base="$I386_BASE"
    fi
    (cd "$root" && fakeroot cpio --extract --quiet --no-absolute-filenames < "$base")
    case "$root" in
        "$scratch"/*) rm -rf -- "$root/opt/scorecard" ;;
        *) echo "unsafe scratch root: $root" >&2; exit 2 ;;
    esac
    mkdir -p "$root/opt/g001-embench"
    install -m 0755 "$INIT" "$root/init"
    install -m 0644 "$CONTRACT" "$root/opt/g001-embench/contract.json"
    for benchmark in "${benchmarks[@]}"; do
        install -m 0755 "$OUTPUT/binaries/$suffix/$benchmark" \
            "$root/opt/g001-embench/$benchmark"
    done
    find "$root" -exec touch -h -d '@0' {} +
    (cd "$root" && find . -print0 | LC_ALL=C sort -z \
        | fakeroot cpio --null --create --format=newc --reproducible 2>/dev/null) \
        > "$OUTPUT/interpreter-g001-embench-$suffix.cpio"
done

cp "$CONTRACT" "$OUTPUT/contract.json"
{
    printf 'upstream_commit=%s\n' "$UPSTREAM_COMMIT"
    printf 'upstream_git_archive_sha256=%s\n' "$UPSTREAM_ARCHIVE_SHA256"
    printf 'zig_version=%s\n' "$ZIG_VERSION"
    printf 'zig_sha256=%s\n' "$ZIG_SHA256"
    printf 'build_executed_guest_binaries=false\n'
    printf 'benchmark_count=%d\n' "${#benchmarks[@]}"
} > "$OUTPUT/BUILD-MANIFEST.txt"

sha256sum \
    "$ZIG" \
    "$RV64_BASE" \
    "$I386_BASE" \
    "$INIT" \
    "$CONTRACT" \
    "$HERE/prepare-interpreter-g001-embench.sh" \
    "$OUTPUT/BUILD-COMMANDS.txt" \
    "$OUTPUT/BUILD-MANIFEST.txt" \
    "$OUTPUT"/binaries/rv64/* \
    "$OUTPUT"/binaries/i386/* \
    "$OUTPUT/interpreter-g001-embench-rv64.cpio" \
    "$OUTPUT/interpreter-g001-embench-i386.cpio" \
    > "$OUTPUT/SHA256SUMS"

file "$OUTPUT"/binaries/rv64/* "$OUTPUT"/binaries/i386/*
echo "sealed unexecuted G001 Embench population under $OUTPUT"
cat "$OUTPUT/SHA256SUMS"
