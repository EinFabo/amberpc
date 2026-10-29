#!/usr/bin/env bash
# Publishes a release: tests, builds and signs the installer on this PC, then creates the
# GitHub release with the installer, its signature and the latest.json the installed apps
# check for updates. The signing key never leaves ~/.amberpc.
#
#   tools/release.sh "What changed, in a sentence or a few lines."
#
# Before: bump "version" in package.json and [workspace.package] in Cargo.toml, commit, push.
set -euo pipefail
cd "$(dirname "$0")/.."
NOTES="${1:?usage: tools/release.sh \"release notes\"}"
REPO="EinFabo/amberpc"

VERSION="$(python3 -I -c 'import json; print(json.load(open("package.json"))["version"])')"
CARGO_VERSION="$(sed -n '/\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)"
TAG="v$VERSION"
[ "$VERSION" = "$CARGO_VERSION" ] || { echo "package.json says $VERSION, Cargo.toml says $CARGO_VERSION"; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo "uncommitted changes - commit first"; exit 1; }
git fetch -q origin main
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] || { echo "HEAD is not origin/main - push first"; exit 1; }
if gh release view "$TAG" -R "$REPO" >/dev/null 2>&1; then echo "$TAG already released"; exit 1; fi

echo "== tests"
cargo test -p amber-core --quiet
tools/win.sh test > /dev/null || { echo "Windows tests failed - run tools/win.sh test"; exit 1; }
BUNDLE="/mnt/c/Users/fabia/build/amberpc/target/release/bundle/nsis"
SETUP="Amber_${VERSION}_x64-setup.exe"
echo "== build $TAG"
# An installer left over from an earlier build must never go out under this tag.
rm -f "$BUNDLE/$SETUP" "$BUNDLE/$SETUP.sig"
LOG="$(mktemp)"
tools/win.sh build > "$LOG" 2>&1 || { tail -40 "$LOG"; echo "build failed"; exit 1; }
rm -f "$LOG"
[ -f "$BUNDLE/$SETUP" ] && [ -f "$BUNDLE/$SETUP.sig" ] || { echo "installer or signature missing in $BUNDLE"; exit 1; }
OUT="$(mktemp -d)"
cp "$BUNDLE/$SETUP" "$BUNDLE/$SETUP.sig" "$OUT/"

echo "== Stream Deck plugin"
PLUGIN="com.einfabo.amber.streamDeckPlugin"
( cd streamdeck && npm ci --silent && npm ci --silent --prefix com.einfabo.amber.sdPlugin && npm test --silent && npm run --silent pack ) > "$OUT/plugin.log" 2>&1 \
  || { tail -30 "$OUT/plugin.log"; echo "plugin build failed"; exit 1; }
cp "streamdeck/dist/$PLUGIN" "$OUT/"

# What the Tauri updater reads: version, notes, and per platform the installer URL with
# its minisign signature.
python3 -I - "$VERSION" "$NOTES" "$OUT/$SETUP.sig" "https://github.com/$REPO/releases/download/$TAG/$SETUP" > "$OUT/latest.json" <<'PY'
import json, sys, datetime
version, notes, sig_path, url = sys.argv[1:5]
entry = {"signature": open(sig_path).read().strip(), "url": url}
print(json.dumps({
    "version": version,
    "notes": notes,
    "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "platforms": {"windows-x86_64": entry, "windows-x86_64-nsis": entry},
}, indent=2))
PY

echo "== publish $TAG"
gh release create "$TAG" -R "$REPO" --target main --title "Amber $TAG" \
  --notes "$NOTES

Download \`$SETUP\` below and run it. Installed copies update themselves.
\`$PLUGIN\` is the Stream Deck plugin - double-click installs it." \
  "$OUT/$SETUP" "$OUT/$SETUP.sig" "$OUT/latest.json" "$OUT/$PLUGIN"
rm -rf "$OUT"
echo "done: https://github.com/$REPO/releases/tag/$TAG"
