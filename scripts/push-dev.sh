#!/usr/bin/env bash
# A local push stack for Klimper in the iOS simulator (clapandpay TASK-087, docs/push.md):
# Xchonnect's reference relay and push gateway on loopback, the example website to pair
# with, and the build settings that point a debug build at this gateway.
#
#   scripts/push-dev.sh                 # start everything, then follow the printed steps
#   scripts/push-dev.sh --no-dapp       # relay and gateway only (you have a website already)
#
# Without an Apple key the gateway can open the sealed token but has nobody to hand it to,
# so this script stands in for Apple: when the gateway has taken a wake-up it delivers the
# same notification to the simulator with `xcrun simctl push`. With a key in the
# environment (XCHONNECT_GATEWAY_APNS_TEAM_ID, _KEY_ID, _KEY_FILE) the gateway talks to
# Apple's sandbox itself, which a simulator on an Apple-silicon Mac receives for real.
#
# Settings: XCHONNECT_DIR (the xchonnect checkout), PUSH_DEV_SIM (simulator name, default
# "iPhone 17"), PUSH_DEV_RELAY_PORT and PUSH_DEV_GATEWAY_PORT (18795, 18796).
# Development only: plain HTTP on loopback, development keys.
set -euo pipefail
cd "$(dirname "$0")/.."
KLIMPER=$PWD

XCHONNECT_DIR=${XCHONNECT_DIR:-$KLIMPER/../../../xchonnect/xchonnect}
SIM=${PUSH_DEV_SIM:-iPhone 17}
RELAY_PORT=${PUSH_DEV_RELAY_PORT:-18795}
GATEWAY_PORT=${PUSH_DEV_GATEWAY_PORT:-18796}
RELAY="http://127.0.0.1:${RELAY_PORT}"
GATEWAY="http://127.0.0.1:${GATEWAY_PORT}"
BUNDLE_ID=com.clapandpay.klimper
DAPP=1
[ "${1:-}" = --no-dapp ] && DAPP=0

[ -d "$XCHONNECT_DIR/crates/gateway" ] || {
  echo "no xchonnect checkout at $XCHONNECT_DIR; set XCHONNECT_DIR" >&2
  exit 1
}

PIDS=()
cleanup() { for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null || true; done; }
trap cleanup EXIT INT TERM
LOGS=$(mktemp -d "${TMPDIR:-/tmp}/klimper-push-dev.XXXXXX")
wait_for() { # wait_for <url> <tries of 0.25 s>
  for _ in $(seq 1 "$2"); do curl -fsS -o /dev/null "$1" 2>/dev/null && return 0; sleep 0.25; done
  return 1
}
counter() { curl -fsS "$GATEWAY/metrics" | awk -v n="xchonnect_gateway_$1_total" '$1 == n { print $2 }'; }

echo "==> building the relay and the gateway ($XCHONNECT_DIR)"
(cd "$XCHONNECT_DIR" && cargo build -q --locked -p xchonnect-relay -p xchonnect-gateway)

echo "==> relay on $RELAY"
# Proof-of-work mailbox creation at a low difficulty; every gateway URL accepted, loopback
# and plain HTTP included, which the relay only allows while it listens on loopback itself.
env XCHONNECT_LISTEN="127.0.0.1:${RELAY_PORT}" XCHONNECT_CREATION=pow \
  XCHONNECT_POW_DIFFICULTY=12 XCHONNECT_GATEWAY_POLICY=open \
  XCHONNECT_DEV_ALLOW_INSECURE_GATEWAYS=true XCHONNECT_OHTTP=ephemeral XCHONNECT_LOG=info \
  "$XCHONNECT_DIR/target/debug/xchonnect-relay" >"$LOGS/relay.log" 2>&1 &
PIDS+=($!)
wait_for "$RELAY/healthz" 40 || { echo "the relay did not start; see $LOGS/relay.log" >&2; exit 1; }

echo "==> gateway on $GATEWAY"
# One key per run is fine: the public key is compiled into the build made from this run.
GATEWAY_KEY_FILE="$LOGS/gateway.key"
if [ -z "${XCHONNECT_GATEWAY_KEYS:-}" ]; then
  XCHONNECT_GATEWAY_KEYS=$(openssl rand -base64 32 | tr '+/' '-_' | tr -d '=\n')
fi
export XCHONNECT_GATEWAY_KEYS
APPLE=0
if [ -n "${XCHONNECT_GATEWAY_APNS_TEAM_ID:-}" ]; then
  APPLE=1
  export XCHONNECT_GATEWAY_APNS_TOPIC=${XCHONNECT_GATEWAY_APNS_TOPIC:-$BUNDLE_ID}
  # A debug build registers with Apple's sandbox.
  export XCHONNECT_GATEWAY_APNS_ENV=${XCHONNECT_GATEWAY_APNS_ENV:-sandbox}
fi
env XCHONNECT_GATEWAY_LISTEN="127.0.0.1:${GATEWAY_PORT}" XCHONNECT_LOG=info \
  "$XCHONNECT_DIR/target/debug/xchonnect-gateway" >"$LOGS/gateway.log" 2>&1 &
PIDS+=($!)
wait_for "$GATEWAY/healthz" 40 || { echo "the gateway did not start; see $LOGS/gateway.log" >&2; exit 1; }
PUBLIC_KEY=$(curl -fsS "$GATEWAY/v1/keys" | sed -n 's/.*"keys":\["\([^"]*\)".*/\1/p')
[ -n "$PUBLIC_KEY" ] || { echo "no public key from $GATEWAY/v1/keys" >&2; exit 1; }
: >"$GATEWAY_KEY_FILE"

ENV_FILE="$KLIMPER/.push-dev.env"
cat >"$ENV_FILE" <<EOF
KLIMPER_PUSH_GATEWAY_URL=$GATEWAY/v1/wake
KLIMPER_PUSH_GATEWAY_KEY=$PUBLIC_KEY
EOF

if [ "$DAPP" = 1 ]; then
  echo "==> example website on http://localhost:5173 (relay $RELAY)"
  (
    cd "$XCHONNECT_DIR"
    [ -f sdk-ts/wasm/xchonnect_bg.wasm ] || ./scripts/build-wasm.sh
    [ -d node_modules ] || npm install --no-fund --no-audit >/dev/null
    [ -d sdk-ts/dist ] || npm run -s build -w @xchonnect/dapp
    VITE_RELAY="$RELAY" exec npm run -s dev -w xchonnect-example-dapp
  ) >"$LOGS/dapp.log" 2>&1 &
  PIDS+=($!)
fi

cat <<EOF

Relay    $RELAY        log $LOGS/relay.log
Gateway  $GATEWAY        log $LOGS/gateway.log   (public key $PUBLIC_KEY)
Website  http://localhost:5173           log $LOGS/dapp.log

1. Build Klimper for the simulator with this gateway compiled in (another terminal):

     set -a; . $ENV_FILE; set +a
     pnpm tauri ios dev "$SIM"

2. In the simulator: create or open a wallet, then Websites (or Settings) -> connect.
   On the website click "Connect wallet", copy the xchonnect: URI, paste it into Klimper's
   pairing field (the Mac's clipboard is the simulator's), compare the six digits, confirm
   both sides. Klimper asks for notification permission now: allow it.

3. Put Klimper in the background (Home) or lock the simulator, then send a request from
   the website. Watch here:
EOF
if [ "$APPLE" = 1 ]; then
  echo "   The gateway sends to Apple's sandbox; delivered_total below counts what Apple accepted."
else
  echo "   No Apple key: this script delivers the gateway's notification to the simulator itself."
fi
echo

# Follow the gateway's counters and, without Apple, stand in for it.
UDID=$(xcrun simctl list devices booted | sed -n "s/^ *${SIM} (\([0-9A-F-]*\)) (Booted).*/\1/p" | head -1)
PAYLOAD="$LOGS/wake.apns"
cat >"$PAYLOAD" <<EOF
{"aps":{"alert":{"title":"Signing request","body":"Open your wallet to review it."},"interruption-level":"time-sensitive","mutable-content":1,"sound":"default"}}
EOF
last_requests=$(counter requests)
while sleep 1; do
  requests=$(counter requests) || break
  [ "$requests" = "$last_requests" ] && continue
  last_requests=$requests
  sleep 0.5 # delivery runs after the gateway has answered the relay
  printf '%s wake-up: requests %s invalid %s limited %s delivered %s failed %s\n' \
    "$(date +%T)" "$requests" "$(counter invalid)" "$(counter limited)" "$(counter delivered)" "$(counter failed)"
  if [ "$APPLE" = 0 ]; then
    [ -n "$UDID" ] || UDID=$(xcrun simctl list devices booted | sed -n "s/^ *${SIM} (\([0-9A-F-]*\)) (Booted).*/\1/p" | head -1)
    if [ -n "$UDID" ]; then
      xcrun simctl push "$UDID" "$BUNDLE_ID" "$PAYLOAD" && echo "          delivered to the simulator $SIM"
    else
      echo "          no booted simulator named '$SIM' to deliver to (PUSH_DEV_SIM)"
    fi
  fi
done
