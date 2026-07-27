#!/usr/bin/env bash
# Modern Debian + RTC + DHCP + in-process HTTP proxy integration test.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
disk="${DEBIAN_DISK:-$root/target/bench/deb-riscv64.ext4}"
[ -f "$disk" ] || {
    echo "missing $disk"
    echo "build it with: nix develop -c tests/vs-v86/mk-debian-rootfs.sh target/bench"
    exit 2
}

work="$(mktemp -d)"
vm_pid=
http_pid=
cleanup() {
    [ -z "$vm_pid" ] || kill "$vm_pid" 2>/dev/null || true
    [ -z "$http_pid" ] || kill "$http_pid" 2>/dev/null || true
    if [ -n "${KEEP_WORK:-}" ]; then
        echo "[virt-proxy] kept diagnostics in $work"
    else
        rm -rf "$work"
    fi
}
trap cleanup EXIT

image="$(nix build --no-link --print-out-paths "$root#virt-kernel" \
    | xargs -I{} find {} -maxdepth 2 -name Image 2>/dev/null | head -1)"
fw="$(nix build --no-link --print-out-paths "$root#virt-opensbi" \
    | xargs -I{} find {} -name fw_dynamic.bin 2>/dev/null | grep -E 'generic' | head -1)"
[ -n "$image" ] && [ -n "$fw" ] || { echo "failed to resolve kernel/OpenSBI"; exit 2; }

(cd "$root" && cargo build --release --bin rv64-vboot >/dev/null)

python3 "$here/server.py" "$work/port" &
http_pid=$!
for _ in $(seq 1 100); do [ -s "$work/port" ] && break; sleep 0.05; done
[ -s "$work/port" ] || { echo "loopback origin failed to start"; exit 1; }
port="$(cat "$work/port")"

mkfifo "$work/in"
exec 3<>"$work/in"
timeout 300 "$root/target/release/rv64-vboot" "$fw" "$image" \
    --disk "$disk" --proxy --ram 1 \
    -- "console=ttyS0 root=/dev/vda rw init=/binit.sh" \
    <"$work/in" >"$work/out" 2>&1 &
vm_pid=$!

wait_for() {
    local marker="$1"
    for _ in $(seq 1 3000); do
        grep -qa "$marker" "$work/out" && return 0
        kill -0 "$vm_pid" 2>/dev/null || return 1
        sleep 0.1
    done
    return 1
}

if ! wait_for BENCH_READY; then
    echo "[virt-proxy] Debian did not boot"
    tail -80 "$work/out"
    exit 1
fi

# Split marker literals so tty input echo cannot satisfy wait_for/checks before
# the command actually executes.
printf '%s\n' \
    'echo CMD_"START"' \
    'now=$(date +%s); if [ "$now" -gt 1700000000 ]; then echo RTC_"USER_OK":$now; else echo RTC_"USER_BAD":$now; fi' \
    '/usr/sbin/udhcpc -i eth0 -n -q -s /usr/share/udhcpc/default.script && echo DHCP_"OK" || { echo DHCP_"FAIL"; ip link set eth0 up; ip addr replace 10.0.2.15/24 dev eth0; ip route replace default via 10.0.2.2; }' \
    'ip -color=never -4 addr show dev eth0; ip -color=never -4 addr show dev eth0 | grep -q "inet 10.0.2.15/24" && echo IP_"OK"' \
    'curl --version | head -1' \
    "body=\$(curl -fsS --connect-timeout 5 --max-time 10 --proxy http://10.0.2.2:8080 http://127.0.0.1:$port/); [ \"\$body\" = DEBIAN_PROXY_OK ] && echo CURL_\"PROXY_OK\"" \
    "body=\$(curl -fsS --http1.1 --connect-timeout 5 --max-time 10 --proxy http://10.0.2.2:8080 -H 'Transfer-Encoding: chunked' --data-binary 'chunked guest body' http://127.0.0.1:$port/chunked); [ \"\$body\" = DEBIAN_CHUNKED_OK ] && echo CURL_\"CHUNKED_OK\"" \
    'test -s /run/rv64-proxy/ca.der && openssl x509 -inform DER -in /run/rv64-proxy/ca.der -noout -subject -issuer | grep -q "rv64.js ephemeral proxy CA" && echo CA_9P_"OK"' \
    'body=$(curl -fsS --noproxy "" --connect-timeout 10 --max-time 30 --proxy http://10.0.2.2:8080 http://example.com/); case "$body" in *"Example Domain"*) echo CURL_HTTP_"EXAMPLE_OK";; esac' \
    'body=$(curl -fsS -v --noproxy "" --connect-timeout 10 --max-time 30 --proxy http://10.0.2.2:8080 https://example.com/ 2>/tmp/rv64-https.trace); case "$body" in *"Example Domain"*) grep -E "subject:|issuer:|SSL certificate verify ok" /tmp/rv64-https.trace; grep -q "rv64.js ephemeral proxy CA" /tmp/rv64-https.trace && grep -q "SSL certificate verify ok" /tmp/rv64-https.trace && echo CURL_TLS_"MITM_OK";; esac' \
    'echo CMD_"DONE"' >&3

if ! wait_for CMD_DONE; then
    echo "[virt-proxy] guest commands did not complete"
    tail -100 "$work/out"
    exit 1
fi

printf '\001x' >&3 || true
wait "$vm_pid" || true
vm_pid=

grep -aE 'PROXY_CA_|BENCH_READY|CMD_START|RTC_USER_|udhcpc:|DHCP_|inet 10\.0\.2|IP_|curl [0-9]|subject:|issuer:|SSL certificate verify ok|CURL_PROXY_OK|CURL_CHUNKED_OK|CA_9P_OK|CURL_HTTP_EXAMPLE_OK|CURL_TLS_MITM_OK|CMD_DONE' "$work/out" || true

failed=0
for marker in PROXY_CA_READY RTC_USER_OK DHCP_OK IP_OK CURL_PROXY_OK CURL_CHUNKED_OK CA_9P_OK CURL_HTTP_EXAMPLE_OK CURL_TLS_MITM_OK; do
    if ! grep -qa "$marker" "$work/out"; then
        echo "[virt-proxy] missing marker: $marker"
        failed=1
    fi
done
if [ "$failed" -ne 0 ]; then
    grep -aE '\[vio\]|\[plic\]|\[vboot\] net|\[hb\]' "$work/out" | tail -200 || true
    tail -120 "$work/out"
    exit 1
fi
echo "[virt-proxy] PASS"
