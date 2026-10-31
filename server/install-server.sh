#!/bin/sh
# Amber Server - speech to text for Amber on the phone, installed in one command:
#
#   curl -fsSL https://github.com/EinFabo/amberpc/releases/download/server-latest/install-server.sh | sh
#
# Picks the build for this machine (Vulkan GPU, CPU with AVX2, or older CPU), checks it
# against the release's SHA256SUMS, installs it to ~/.local/bin, sets it up with the
# recommended model, starts it as a service and shows the phone's address. Run it again to
# update; the settings and models stay.
#
# AMBER_SERVER_FORCE_BUILD=cpu|cpu-compat|vulkan picks the build yourself.
set -eu

# Everything inside main, called on the last line: a download cut off half-way runs nothing.
main() {

BASE="${AMBER_SERVER_BASE:-https://github.com/EinFabo/amberpc/releases/download/server-latest}"
DEST="$HOME/.local/bin"
BIN="$DEST/amber-server"

say() { printf '%s\n' "$*"; }
fail() {
    printf 'amber-server install: %s\n' "$*" >&2
    exit 1
}

case "$(uname -s)-$(uname -m)" in
    Linux-x86_64 | Linux-amd64) ;;
    *) fail "this installer is for Linux on x86-64 (this is $(uname -s) $(uname -m)); Windows: install-server.ps1" ;;
esac

CPUINFO="${AMBER_SERVER_CPUINFO:-/proc/cpuinfo}"
cpu_has() { grep -qw "$1" "$CPUINFO" 2> /dev/null; }

# Every build needs AVX; many NAS and mini-PC processors have none.
if [ -z "${AMBER_SERVER_FORCE_BUILD:-}" ] && ! cpu_has avx; then
    fail "this CPU has no AVX, which Amber Server needs - it cannot run on this machine"
fi

# A GPU the Vulkan build can use: the Vulkan loader and a real GPU.
vulkan_gpu() {
    # Debian keeps ldconfig in /sbin, off a normal user's PATH.
    { ldconfig -p 2> /dev/null || /sbin/ldconfig -p 2> /dev/null || true; } | grep -q 'libvulkan\.so\.1' || return 1
    if command -v vulkaninfo > /dev/null 2>&1; then
        vulkaninfo --summary 2>/dev/null | grep -q 'PHYSICAL_DEVICE_TYPE_DISCRETE_GPU'
    else
        command -v lspci > /dev/null 2>&1 &&
            lspci 2>/dev/null | grep -iE 'vga|3d|display' | grep -qiE 'nvidia|advanced micro devices'
    fi
}

cpu_build=cpu-compat
cpu_has avx2 && cpu_build=cpu
build="${AMBER_SERVER_FORCE_BUILD:-}"
if [ -z "$build" ]; then
    # The Vulkan build is built for AVX2 too.
    if [ "$cpu_build" = cpu ] && vulkan_gpu; then
        build=vulkan
    else
        build=$cpu_build
    fi
fi
case "$build" in
    cpu | cpu-compat | vulkan) ;;
    *) fail "unknown build '$build' - cpu, cpu-compat or vulkan" ;;
esac

fetch() {
    if command -v curl > /dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    elif command -v wget > /dev/null 2>&1; then
        wget -q "$1" -O "$2"
    else
        fail "needs curl or wget"
    fi
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# Downloads and checks one build; fails with the reason.
download() {
    asset="amber-server-linux-x64-$1"
    say "Downloading $asset ..."
    fetch "$BASE/SHA256SUMS" "$tmp/SHA256SUMS" || fail "could not download $BASE/SHA256SUMS"
    fetch "$BASE/$asset" "$tmp/$asset" || fail "could not download $BASE/$asset"
    grep "  $asset\$" "$tmp/SHA256SUMS" > "$tmp/sum" || fail "SHA256SUMS has no line for $asset"
    (cd "$tmp" && sha256sum -c --status sum) || fail "$asset does not match its checksum - not installed"
    chmod 755 "$tmp/$asset"
}

download "$build"
# The Vulkan build needs a working driver: it says whether it sees a GPU.
if [ "$build" = vulkan ] && ! "$tmp/amber-server-linux-x64-vulkan" gpu-check > /dev/null 2>&1; then
    say "The GPU build sees no usable GPU here (driver?) - taking the CPU build."
    build=$cpu_build
    download "$build"
fi

mkdir -p "$DEST"
# Replaced in one step: a running server keeps its old file until it restarts.
cp "$tmp/amber-server-linux-x64-$build" "$DEST/.amber-server.new"
mv -f "$DEST/.amber-server.new" "$BIN"
say "Installed $BIN ($build build)"
say ""

"$BIN" setup --quick
say ""
if "$BIN" service install; then
    # Up and answering? Loading the model takes a moment.
    waited=0
    until "$BIN" status 2> /dev/null | grep -q "answers on port"; do
        if [ "$waited" -ge "${AMBER_SERVER_WAIT:-20}" ]; then
            say ""
            say "The server was installed but does not answer. Its last lines:"
            "$BIN" logs 2>&1 | tail -n 5
            fail "see above - amber-server status and amber-server logs tell more"
        fi
        sleep 1
        waited=$((waited + 1))
    done
else
    say ""
    say "The background service could not be set up - start the server yourself: $BIN run"
fi
say ""
"$BIN" url
say ""
say "Optional - tidier text with a local language model (a large download): amber-server cleanup add"
case ":$PATH:" in
    *":$DEST:"*) ;;
    *) say "To type amber-server anywhere, add $DEST to your PATH: echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.profile" ;;
esac
}

main "$@"
