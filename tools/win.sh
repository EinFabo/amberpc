#!/usr/bin/env bash
# Builds and runs Amber on the Windows side. The sources live in WSL; Windows tools work
# badly on \\wsl$ paths, so the tree is mirrored to C:\Users\fabia\build\amberpc first.
#
#   tools/win.sh install     npm ci
#   tools/win.sh check       cargo check (Windows target)
#   tools/win.sh test        cargo test for the whole workspace on Windows
#   tools/win.sh build       tauri build -> NSIS installer
#   tools/win.sh dev         tauri dev
#   tools/win.sh <anything>  run it in the mirrored folder
set -euo pipefail
SRC="$(cd "$(dirname "$0")/.." && pwd)"
WIN_DIR='C:\Users\fabia\build\amberpc'
DEST="/mnt/c/Users/fabia/build/amberpc"
mkdir -p "$DEST"
rsync -a --delete \
  --exclude .git --exclude node_modules --exclude target --exclude dist \
  --exclude src-tauri/gen --exclude src-tauri/icons --exclude '*.log' \
  "$SRC/" "$DEST/"
# Generated icons live only on the Windows side, unless they exist here.
if [ -d "$SRC/src-tauri/icons" ]; then rsync -a "$SRC/src-tauri/icons/" "$DEST/src-tauri/icons/"; fi

case "${1:-check}" in
  install) CMD="npm ci" ;;
  check) CMD="cargo check -p amber-pc --message-format short" ;;
  test) CMD="cargo test --workspace" ;;
  build)
    # The updater signs the installer; key and password stay in ~/.amberpc, outside the repo.
    KEY_WIN="$(wslpath -w "$HOME/.amberpc/updater.key")"
    PW="$(cat "$HOME/.amberpc/updater.password")"
    CMD="\$env:TAURI_SIGNING_PRIVATE_KEY='$KEY_WIN'; \$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD='$PW'; npm run tauri build" ;;
  dev) CMD="npm run app" ;;
  *) CMD="$*" ;;
esac
cd /mnt/c && powershell.exe -NoProfile -Command "Set-Location '$WIN_DIR'; \$env:CARGO_TERM_COLOR='never'; $CMD; exit \$LASTEXITCODE" 2>&1 | tr -d '\r'
