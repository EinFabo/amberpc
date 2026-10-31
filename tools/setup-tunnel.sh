#!/usr/bin/env bash
# One command to reach Amber's phone API from anywhere, through a Cloudflare Tunnel on your
# own domain (run in WSL on the PC where Amber is installed):
#
#   tools/setup-tunnel.sh amber-x7k2.example.com
#
# Logs in to Cloudflare once (a browser window: pick the domain), creates the tunnel "amber",
# points the hostname at it, and hands token and hostname to Amber with a new access key -
# the token never shows on screen or on a command line. Run it again with another hostname
# to move the address; --keep-key keeps the phone's current key.
set -euo pipefail

HOST="${1:-}"
NEW_KEY=true
[ "${2:-}" = "--keep-key" ] && NEW_KEY=false
if [ -z "$HOST" ] || [ "${HOST#-}" != "$HOST" ]; then
  echo "usage: tools/setup-tunnel.sh <hostname> [--keep-key]"
  echo "  e.g. tools/setup-tunnel.sh amber-$(tr -dc 'a-z0-9' </dev/urandom | head -c 6).your-domain.com"
  echo "  (a name that is hard to guess; the domain must be on your Cloudflare account)"
  exit 2
fi
NAME="${AMBER_TUNNEL_NAME:-amber}"

win() { cmd.exe /c "echo %$1%" 2>/dev/null | tr -d '\r'; }
LOCAL="$(wslpath -u "$(win LOCALAPPDATA)")"
PROFILE="$(wslpath -u "$(win USERPROFILE)")"
AMBER="$LOCAL/Amber/amber.exe"
LOG="$LOCAL/app.amber.pc/logs/amber.log"
CF="$(ls "$LOCAL"/app.amber.pc/tools/cloudflared-*.exe 2>/dev/null | sort -V | tail -1 || true)"
[ -x "$AMBER" ] || { echo "Amber is not installed on this PC."; exit 1; }
[ -n "$CF" ] || { echo "cloudflared is missing: in Amber, Phone -> From anywhere -> switch on -> Download."; exit 1; }

if [ ! -f "$PROFILE/.cloudflared/cert.pem" ]; then
  echo "== Cloudflare login: a browser window opens - pick your domain and authorize."
  "$CF" tunnel login
fi
if "$CF" tunnel info "$NAME" >/dev/null 2>&1; then
  echo "== Tunnel \"$NAME\" is there already."
else
  echo "== Creating the tunnel \"$NAME\"."
  "$CF" tunnel create "$NAME" >/dev/null
fi
echo "== $HOST -> tunnel \"$NAME\"."
"$CF" tunnel route dns --overwrite-dns "$NAME" "$HOST"

# Token and hostname go to Amber in a file in the user's own temp folder, which Amber deletes
# as soon as it has read it.
HANDOVER_WIN="$(win TEMP)\\amber-tunnel-$$.json"
HANDOVER="$(wslpath -u "$HANDOVER_WIN")"
trap 'rm -f "$HANDOVER"' EXIT
TOKEN="$("$CF" tunnel token "$NAME" | tr -d '\r\n')"
printf '{"token":"%s","host":"%s","newKey":%s}' "$TOKEN" "$HOST" "$NEW_KEY" >"$HANDOVER"
unset TOKEN
LINES_BEFORE="$(wc -l <"$LOG" 2>/dev/null || echo 0)"
"$AMBER" --import-tunnel "$HANDOVER_WIN" >/dev/null 2>&1 &

echo "== Handing it to Amber..."
for _ in $(seq 1 60); do
  sleep 1
  NEW="$(tail -n +"$((LINES_BEFORE + 1))" "$LOG" 2>/dev/null || true)"
  if grep -q "tunnel setup failed" <<<"$NEW"; then
    grep "tunnel setup failed" <<<"$NEW" | tail -1
    exit 1
  fi
  if grep -q "tunnel connected" <<<"$NEW"; then
    echo "== Connected."
    break
  fi
done
grep -q "tunnel connected" <<<"${NEW:-}" || { echo "Amber has not reported the tunnel as connected yet - see Phone -> From anywhere."; exit 1; }

# Cloudflare needs a moment for a new name.
for _ in $(seq 1 20); do
  if curl -fsS --max-time 5 "https://$HOST/health" >/dev/null 2>&1; then
    echo "== https://$HOST answers. Amber shows the phone's address and QR code under Phone -> From anywhere."
    $NEW_KEY && echo "   The phone needs that new address (with a new key): in Amber on the phone, Engine -> Own server."
    exit 0
  fi
  sleep 3
done
echo "The tunnel is connected, but https://$HOST does not answer yet - DNS may take a few minutes. Try: curl https://$HOST/health"
