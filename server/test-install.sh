#!/bin/sh
# Tests install-server.sh against a fake release: sh server/test-install.sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
failures=0
fail() {
    printf 'FAIL: %s\n' "$*"
    failures=$((failures + 1))
}

# A release with a stub per build that writes down how it was called.
release() {
    rm -rf "${tmp:?}/release" "${tmp:?}/home" "${tmp:?}/calls"
    mkdir -p "$tmp/release" "$tmp/home"
    for build in cpu cpu-compat vulkan; do
        cat > "$tmp/release/amber-server-linux-x64-$build" << STUB
#!/bin/sh
case "\$1" in
    --version) echo "amber-server 0.0.0 $build"; exit 0 ;;
    gpu-check) [ -z "\${STUB_NO_GPU:-}" ] && exit 0; exit 1 ;;
    status) [ -n "\${STUB_DOWN:-}" ] && echo "server   not running" || echo "server   answers on port 8765"; exit 0 ;;
    logs) echo "amber-server: port 8765 is in use"; exit 0 ;;
esac
echo "\$*" >> "$tmp/calls"
[ "\$*" = "service install" ] && [ -n "\${STUB_NO_SYSTEMD:-}" ] && exit 1
exit 0
STUB
    done
    (cd "$tmp/release" && sha256sum amber-server-linux-x64-* > SHA256SUMS)
}

# A machine: CPU flags, and whether a Vulkan GPU is there (fake ldconfig and vulkaninfo).
machine() {
    printf 'processor\t: 0\nflags\t\t: fpu sse sse2 %s\n' "$1" > "$tmp/cpuinfo"
    rm -rf "${tmp:?}/bin"
    mkdir -p "$tmp/bin"
    if [ "$2" = gpu ]; then
        printf '#!/bin/sh\necho "\tlibvulkan.so.1 (libc6,x86-64) => /usr/lib/libvulkan.so.1"\n' > "$tmp/bin/ldconfig"
        printf '#!/bin/sh\necho "deviceType = PHYSICAL_DEVICE_TYPE_DISCRETE_GPU"\n' > "$tmp/bin/vulkaninfo"
    else
        printf '#!/bin/sh\nexit 0\n' > "$tmp/bin/ldconfig"
    fi
    chmod +x "$tmp/bin/"*
}

install() {
    PATH="$tmp/bin:$PATH" AMBER_SERVER_BASE="file://$tmp/release" AMBER_SERVER_CPUINFO="$tmp/cpuinfo" \
        AMBER_SERVER_WAIT=1 HOME="$tmp/home" sh "$here/install-server.sh" > "$tmp/out" 2>&1
}

installed() { "$tmp/home/.local/bin/amber-server" --version 2> /dev/null | awk '{print $3}'; }

# 1. Installs, sets up, starts the service, checks it answers, shows the address.
release
machine "avx avx2" none
if AMBER_SERVER_FORCE_BUILD=cpu-compat install; then
    [ "$(installed)" = cpu-compat ] || fail "not installed as asked: $(installed)"
    calls=$(tr '\n' ',' < "$tmp/calls" 2> /dev/null || true)
    [ "$calls" = "setup --quick,service install,url," ] || fail "the commands were: $calls"
    grep -q "amber-server cleanup add" "$tmp/out" || fail "no hint about the cleanup"
else
    fail "the install failed: $(cat "$tmp/out")"
fi

# 2. Run again as an update: works, sets up again (which keeps the choices).
if install; then
    [ "$(grep -c "setup --quick" "$tmp/calls")" -eq 2 ] || fail "the update did not run setup"
else
    fail "the update failed: $(cat "$tmp/out")"
fi

# 3. A wrong checksum installs nothing.
release
sed -i 's/^[0-9a-f]\{64\}/0000000000000000000000000000000000000000000000000000000000000000/' "$tmp/release/SHA256SUMS"
if AMBER_SERVER_FORCE_BUILD=cpu-compat install; then fail "installed despite a wrong checksum"; fi
[ ! -e "$tmp/home/.local/bin/amber-server" ] || fail "a binary was installed despite a wrong checksum"
[ ! -e "$tmp/calls" ] || fail "the binary ran despite a wrong checksum"

# 4. Without systemd the install still ends with the address and how to start by hand.
release
if STUB_NO_SYSTEMD=1 install; then
    grep -q "amber-server run" "$tmp/out" || fail "no hint to start it by hand: $(cat "$tmp/out")"
    tail -n 1 "$tmp/calls" | grep -qx "url" || fail "the address was not shown"
else
    fail "the install failed without systemd: $(cat "$tmp/out")"
fi

# 5. The build follows the machine.
release
machine "avx avx2 fma" gpu
install || fail "GPU machine: $(cat "$tmp/out")"
[ "$(installed)" = vulkan ] || fail "a GPU with AVX2 should get vulkan, got $(installed)"
release
machine "avx" gpu
install || fail "GPU, old CPU: $(cat "$tmp/out")"
[ "$(installed)" = cpu-compat ] || fail "no AVX2 must not get vulkan (needs AVX2), got $(installed)"
release
machine "avx avx2" gpu
STUB_NO_GPU=1 install || fail "GPU without driver: $(cat "$tmp/out")"
[ "$(installed)" = cpu ] || fail "the GPU build seeing no GPU should give way to cpu, got $(installed)"
release
machine "avx avx2" none
install || fail "CPU machine: $(cat "$tmp/out")"
[ "$(installed)" = cpu ] || fail "AVX2 without GPU should get cpu, got $(installed)"

# 6. A CPU without AVX is told, and nothing is installed.
release
machine "sse4_2" none
if install; then fail "installed on a CPU without AVX"; fi
grep -q "AVX" "$tmp/out" || fail "no word about AVX: $(cat "$tmp/out")"
[ ! -e "$tmp/home/.local/bin/amber-server" ] || fail "installed on a CPU without AVX"

# 7. A server that does not answer after the install is reported, with its log.
release
machine "avx avx2" none
if STUB_DOWN=1 install; then fail "a server that does not answer counted as installed"; fi
grep -q "port 8765 is in use" "$tmp/out" || fail "the log was not shown: $(cat "$tmp/out")"

if [ "$failures" -eq 0 ]; then echo "install tests: ok"; else exit 1; fi
