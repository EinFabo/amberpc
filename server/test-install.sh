#!/bin/sh
# Tests install-server.sh against a fake release: sh server/test-install.sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
failures=0
fail() { printf 'FAIL: %s\n' "$*"; failures=$((failures + 1)); }

# A release with a stub binary that writes down how it was called.
release() {
    rm -rf "${tmp:?}/release" "${tmp:?}/home" "${tmp:?}/calls"
    mkdir -p "$tmp/release" "$tmp/home"
    cat > "$tmp/release/amber-server-linux-x64-cpu-compat" <<STUB
#!/bin/sh
case "\$1" in --version) echo "amber-server 0.0.0"; exit 0 ;; esac
echo "\$*" >> "$tmp/calls"
[ "\$*" = "service install" ] && [ -n "\${STUB_NO_SYSTEMD:-}" ] && exit 1
exit 0
STUB
    (cd "$tmp/release" && sha256sum amber-server-linux-x64-cpu-compat > SHA256SUMS)
}

install() {
    AMBER_SERVER_BASE="file://$tmp/release" AMBER_SERVER_FORCE_BUILD=cpu-compat HOME="$tmp/home" \
        sh "$here/install-server.sh" > "$tmp/out" 2>&1
}

# 1. Installs, sets up, starts the service, shows the address.
release
if install; then
    [ -x "$tmp/home/.local/bin/amber-server" ] || fail "the binary is not in ~/.local/bin"
    calls=$(tr '\n' ',' < "$tmp/calls" 2>/dev/null || true)
    [ "$calls" = "setup --quick,service install,url," ] || fail "the commands were: $calls"
    grep -q "amber-server cleanup add" "$tmp/out" || fail "no hint about the cleanup"
else
    fail "the install failed: $(cat "$tmp/out")"
fi

# 2. A wrong checksum installs nothing.
release
echo "0000000000000000000000000000000000000000000000000000000000000000  amber-server-linux-x64-cpu-compat" > "$tmp/release/SHA256SUMS"
if install; then fail "installed despite a wrong checksum"; fi
[ ! -e "$tmp/home/.local/bin/amber-server" ] || fail "a binary was installed despite a wrong checksum"
[ ! -e "$tmp/calls" ] || fail "the binary ran despite a wrong checksum"

# 3. Without systemd the install still ends with the address and how to start by hand.
release
if STUB_NO_SYSTEMD=1 install; then
    grep -q "amber-server run" "$tmp/out" || fail "no hint to start it by hand: $(cat "$tmp/out")"
    tail -n 1 "$tmp/calls" | grep -qx "url" || fail "the address was not shown"
else
    fail "the install failed without systemd: $(cat "$tmp/out")"
fi

if [ "$failures" -eq 0 ]; then echo "install tests: ok"; else exit 1; fi
